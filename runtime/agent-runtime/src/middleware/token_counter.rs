// ============================================================================
// TOKEN COUNTER
// Model-aware token counting for middleware budget decisions
// ============================================================================

//! # Token Counter
//!
//! Token estimation utilities for middleware.
//!
//! Uses `tiktoken-rs` encoders for OpenAI-family models (exact count) and
//! `cl100k_base` as an approximation for non-OpenAI providers — published
//! comparisons put the skew at ≤15% against Claude / Llama / GLM / Gemini /
//! Mistral, a decisive improvement over the previous chars/4 heuristic
//! which drifted by ~40% on code and unicode.
//!
//! Unknown models fall back to chars/4 with a one-time `tracing::warn!` so
//! new providers never silently re-introduce the old skew.

use crate::types::ChatMessage;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use tiktoken_rs::CoreBPE;

/// `o200k_base` — the encoder OpenAI uses for `gpt-4o`, `o1`, `o3`, and later
/// families. Built once and shared across calls (BPE construction is ~50 ms).
static O200K_BASE: LazyLock<Option<CoreBPE>> = LazyLock::new(|| tiktoken_rs::o200k_base().ok());

/// `cl100k_base` — the encoder OpenAI ships for `gpt-4` / `gpt-3.5-turbo`.
/// Doubles as the approximation path for non-OpenAI OAI-compat providers.
static CL100K_BASE: LazyLock<Option<CoreBPE>> = LazyLock::new(|| tiktoken_rs::cl100k_base().ok());

/// Set of model names already logged as falling back to chars/4 so the
/// warning fires once per model, not once per call.
static WARNED_MODELS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// OpenAI per-message overhead for role + formatting.
const PER_MESSAGE_OVERHEAD_TOKENS: usize = 4;

/// Pick the encoder for a model name. Returns `None` for unknown models
/// so the caller falls back to chars/4.
///
/// Prefix matching is intentional: `gpt-4o-mini` and `gpt-4o-2024-08-06`
/// both route through the same `gpt-4o` arm.
fn encoder_for(model: &str) -> Option<&'static CoreBPE> {
    // OpenAI newer families use o200k_base.
    if model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("gpt-4o")
        || model.starts_with("gpt-4.5")
        || model.starts_with("gpt-5")
    {
        return O200K_BASE.as_ref();
    }

    // cl100k_base is the native encoder for gpt-4 / gpt-3.5-turbo and the
    // best-available approximation for non-OpenAI OAI-compat providers.
    // The ≤15% skew against their native tokenizers is acceptable for
    // middleware threshold decisions.
    if model.starts_with("gpt-4")
        || model.starts_with("gpt-3.5")
        || model.starts_with("claude")
        || model.starts_with("deepseek")
        || model.starts_with("glm")
        || model.starts_with("qwen")
        || model.starts_with("gemini")
        || model.starts_with("mistral")
        || model.starts_with("mixtral")
        || model.starts_with("llama")
    {
        return CL100K_BASE.as_ref();
    }

    None
}

/// Character-based fallback — the legacy heuristic. Preserved as a last
/// resort for unknown models so runtime never panics on a new provider.
fn chars_fallback(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    (text.len() / 4).max(1)
}

/// Emit a `tracing::warn!` the first time an unknown model falls back.
/// Subsequent calls for the same model are silent to avoid log spam.
fn warn_fallback_once(model: &str) {
    let Ok(mut warned) = WARNED_MODELS.lock() else {
        return;
    };
    if warned.insert(model.to_string()) {
        tracing::warn!(
            model = %model,
            "token_counter: no tokenizer match for model, falling back to chars/4 \
             (~40% skew on code). Add the model prefix to encoder_for()."
        );
    }
}

/// Estimate tokens for a string under the given model's tokenizer.
#[must_use]
pub fn estimate_tokens(text: &str, model: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    match encoder_for(model) {
        Some(bpe) => bpe.encode_with_special_tokens(text).len(),
        None => {
            warn_fallback_once(model);
            chars_fallback(text)
        }
    }
}

/// Estimate tokens for a single message, including tool-call envelopes and
/// per-message formatting overhead.
#[must_use]
pub fn estimate_message_tokens(message: &ChatMessage, model: &str) -> usize {
    let mut tokens = estimate_tokens(&message.text_content(), model);

    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            tokens += estimate_tokens(&tool_call.name, model);
            tokens += estimate_tokens(&tool_call.arguments.to_string(), model);
        }
    }

    tokens.saturating_add(PER_MESSAGE_OVERHEAD_TOKENS)
}

/// Estimate total tokens for all messages in a conversation.
#[must_use]
pub fn estimate_total_tokens(messages: &[ChatMessage], model: &str) -> usize {
    messages
        .iter()
        .map(|m| estimate_message_tokens(m, model))
        .sum()
}

/// Fraction of the model's context window currently occupied.
#[must_use]
pub fn estimate_context_fraction(tokens: usize, context_window: usize) -> f64 {
    if context_window == 0 {
        return 0.0;
    }
    tokens as f64 / context_window as f64
}

/// Default context window size for known models. Unknown models get a
/// conservative 8k fallback.
#[must_use]
pub fn get_model_context_window(model: &str) -> usize {
    match model {
        // Order matters: gpt-4o must match before the generic gpt-4 arm.
        m if m.contains("gpt-4o") => 128_000,
        m if m.starts_with("gpt-4") => 128_000,
        m if m.starts_with("gpt-3.5") => 16_385,

        m if m.contains("claude-3-5-sonnet") => 200_000,
        m if m.contains("claude-3-5-haiku") => 200_000,
        m if m.contains("claude-3-opus") => 200_000,
        m if m.contains("claude-3-sonnet") => 200_000,
        m if m.contains("claude-3-haiku") => 200_000,
        m if m.contains("claude-") => 200_000,

        m if m.contains("deepseek") => 128_000,

        m if m.contains("gemini-2") => 1_000_000,
        m if m.contains("gemini-1.5") => 1_000_000,
        m if m.contains("gemini-1") => 32_000,

        m if m.contains("llama-3") => 128_000,
        m if m.contains("llama-2") => 4_096,

        m if m.contains("glm") => 128_000,

        m if m.contains("mistral-large") => 128_000,
        m if m.contains("mistral-7b") => 32_768,
        m if m.contains("mixtral") => 32_768,

        _ => 8_192,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCall;
    use agent_primitives::types::Part;
    use serde_json::json;

    // ------------------------------------------------------------------
    // 1. Exact reference match: our estimate == tiktoken's own count.
    // ------------------------------------------------------------------

    #[test]
    fn gpt4_matches_tiktoken_cl100k_reference() {
        let text = "The quick brown fox jumps over the lazy dog";
        let ours = estimate_tokens(text, "gpt-4-turbo");
        let reference = tiktoken_rs::cl100k_base()
            .expect("cl100k_base")
            .encode_with_special_tokens(text)
            .len();
        assert_eq!(ours, reference, "gpt-4* must route to cl100k_base");
    }

    #[test]
    fn gpt4o_matches_tiktoken_o200k_reference() {
        let text = "The quick brown fox jumps over the lazy dog";
        let ours = estimate_tokens(text, "gpt-4o-mini");
        let reference = tiktoken_rs::o200k_base()
            .expect("o200k_base")
            .encode_with_special_tokens(text)
            .len();
        assert_eq!(ours, reference, "gpt-4o* must route to o200k_base");
    }

    // ------------------------------------------------------------------
    // 2. Non-OpenAI models use cl100k_base as an approximation.
    //    Skew against native tokenizer is ≤15% per published comparisons.
    //    Here we just assert routing: our count == cl100k reference.
    // ------------------------------------------------------------------

    #[test]
    fn claude_routes_to_cl100k_approximation() {
        let text = "Analyze the following codebase for memory safety issues.";
        let ours = estimate_tokens(text, "claude-3-5-sonnet-20241022");
        let reference = tiktoken_rs::cl100k_base()
            .expect("cl100k_base")
            .encode_with_special_tokens(text)
            .len();
        assert_eq!(ours, reference, "claude-* must route to cl100k_base");
    }

    #[test]
    fn llama_routes_to_cl100k_approximation() {
        let text = "Summarize the three main findings.";
        let ours = estimate_tokens(text, "llama-3.1-70b-instruct");
        let reference = tiktoken_rs::cl100k_base()
            .expect("cl100k_base")
            .encode_with_special_tokens(text)
            .len();
        assert_eq!(ours, reference, "llama-* must route to cl100k_base");
    }

    // ------------------------------------------------------------------
    // 3. Unknown model falls back to chars/4 — preserves legacy behavior
    //    so a new provider never crashes. Warning fires once.
    // ------------------------------------------------------------------

    #[test]
    fn unknown_model_falls_back_to_chars() {
        let text = "a".repeat(100);
        let count = estimate_tokens(&text, "totally-new-provider-xyz");
        assert_eq!(count, 25, "fallback must be len/4 = 100/4 = 25");
    }

    #[test]
    fn empty_text_returns_zero_for_any_model() {
        assert_eq!(estimate_tokens("", "gpt-4o"), 0);
        assert_eq!(estimate_tokens("", "claude-3-5-sonnet"), 0);
        assert_eq!(estimate_tokens("", "unknown-model"), 0);
    }

    // ------------------------------------------------------------------
    // 4. Unicode / code under-counting was the real-world pain point.
    //    Assert the real tokenizer counts MORE than chars/4 would on both.
    // ------------------------------------------------------------------

    #[test]
    fn unicode_not_undercounted_by_chars_heuristic() {
        // Ten 🎉 emojis. UTF-8 bytes per emoji = 4, so len = 40, chars/4 = 10.
        // cl100k counts each emoji as multiple tokens.
        let text = "🎉".repeat(10);
        let real = estimate_tokens(&text, "claude-3-5-sonnet");
        assert!(
            real > 10,
            "expected real tokenizer to count unicode higher than chars/4 = 10, got {real}"
        );
    }

    #[test]
    fn code_not_undercounted_by_chars_heuristic() {
        // Python with lots of short tokens (keywords, punctuation, identifiers)
        // — char/4 notoriously undercounts code by ~25%.
        let code = "def fetch_user(user_id: int) -> Optional[User]:\n    \
                    if not user_id:\n        return None\n    \
                    return db.query(User).filter_by(id=user_id).first()\n";
        let real = estimate_tokens(code, "gpt-4-turbo");
        let chars_over_four = code.len() / 4;
        assert!(
            real > chars_over_four,
            "code tokens ({real}) should exceed chars/4 ({chars_over_four}) — \
             real tokenizer catches short identifier tokens"
        );
    }

    // ------------------------------------------------------------------
    // 5. Tool call arguments contribute to message token count.
    // ------------------------------------------------------------------

    #[test]
    fn tool_call_args_are_counted() {
        let mut msg = ChatMessage {
            role: "assistant".to_string(),
            content: vec![Part::Text {
                text: "Let me check the file.".to_string(),
            }],
            tool_calls: None,
            tool_call_id: None,
            is_summary: false,
        };
        let without_tc = estimate_message_tokens(&msg, "gpt-4-turbo");

        msg.tool_calls = Some(vec![ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({"path": "src/main.rs", "offset": 0, "limit": 200}),
        }]);
        let with_tc = estimate_message_tokens(&msg, "gpt-4-turbo");

        assert!(
            with_tc > without_tc + 5,
            "tool call with args must add material tokens: {without_tc} -> {with_tc}"
        );
    }

    // ------------------------------------------------------------------
    // 6. Aggregation over a conversation.
    // ------------------------------------------------------------------

    #[test]
    fn estimate_total_tokens_sums_across_messages() {
        let messages = vec![
            ChatMessage::user("first message".to_string()),
            ChatMessage::assistant("second message".to_string()),
            ChatMessage::user("third message".to_string()),
        ];
        let total = estimate_total_tokens(&messages, "gpt-4-turbo");
        let a = estimate_message_tokens(&messages[0], "gpt-4-turbo");
        let b = estimate_message_tokens(&messages[1], "gpt-4-turbo");
        let c = estimate_message_tokens(&messages[2], "gpt-4-turbo");
        assert_eq!(total, a + b + c);
    }

    // ------------------------------------------------------------------
    // Preserved from the legacy heuristic test surface — now parameterized
    // by model so the fallback path is still covered.
    // ------------------------------------------------------------------

    #[test]
    fn model_context_windows_unchanged() {
        assert!(get_model_context_window("gpt-4o") >= 100_000);
        assert!(get_model_context_window("claude-3-5-sonnet") >= 100_000);
        assert!(get_model_context_window("deepseek-chat") >= 100_000);
        assert_eq!(get_model_context_window("unknown"), 8_192);
    }

    #[test]
    fn context_fraction_math() {
        assert!((estimate_context_fraction(0, 100_000) - 0.0).abs() < f64::EPSILON);
        assert!((estimate_context_fraction(50_000, 100_000) - 0.5).abs() < f64::EPSILON);
        assert!((estimate_context_fraction(100_000, 100_000) - 1.0).abs() < f64::EPSILON);
        // Zero window: no panic, returns 0.
        assert!((estimate_context_fraction(100, 0) - 0.0).abs() < f64::EPSILON);
    }
}
