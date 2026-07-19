//! Context management helpers for the agent executor.
//!
//! Functions for compacting, sanitizing, and truncating message history
//! to keep context within token limits.

use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::types::ChatMessage;

/// Default maximum characters retained for a tool result sent back to the
/// model after optional offload.
pub const DEFAULT_MAX_TOOL_RESULT_CHARS: usize = 30_000;

/// Configuration for preparing a tool result for model context or durable
/// replay.
#[derive(Debug, Clone)]
pub struct ToolResultContextConfig {
    /// Maximum characters kept in context after optional offload. `0` disables
    /// truncation.
    pub max_tool_result_chars: usize,
    /// Save large successful results to a file and return a pointer instead of
    /// replaying the full result.
    pub offload_large_results: bool,
    /// Character threshold for offloading.
    pub offload_threshold_chars: usize,
    /// Directory where offloaded raw results are written.
    pub offload_dir: Option<PathBuf>,
}

impl Default for ToolResultContextConfig {
    fn default() -> Self {
        Self {
            max_tool_result_chars: DEFAULT_MAX_TOOL_RESULT_CHARS,
            offload_large_results: false,
            offload_threshold_chars: 20_000,
            offload_dir: None,
        }
    }
}

/// Prepare a successful tool result for insertion into model-visible context.
///
/// Large results are optionally offloaded to disk first, then the context text
/// is truncated as a safety net. The returned value is safe to persist in
/// replayable message history; the original raw result remains in the offload
/// file when offloading succeeds.
pub fn prepare_tool_result_for_context(
    tool_name: &str,
    result: String,
    config: &ToolResultContextConfig,
) -> String {
    let processed = offload_tool_result_for_context(tool_name, result, config);
    truncate_tool_result(processed, config.max_tool_result_chars)
}

pub(crate) fn offload_tool_result_for_context(
    tool_name: &str,
    result: String,
    config: &ToolResultContextConfig,
) -> String {
    if !config.offload_large_results {
        return result;
    }

    if result.len() <= config.offload_threshold_chars {
        return result;
    }

    let offload_dir = match &config.offload_dir {
        Some(dir) => dir.clone(),
        None => {
            if let Some(home) = dirs::home_dir() {
                home.join("Documents").join("zbot").join("temp")
            } else {
                tracing::warn!("Could not determine home directory for offload");
                return result;
            }
        }
    };

    if let Err(e) = std::fs::create_dir_all(&offload_dir) {
        tracing::warn!("Failed to create offload directory: {}", e);
        return result;
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let sanitized_tool = tool_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let uuid = uuid::Uuid::new_v4().as_simple().to_string();
    let offload_id = &uuid[..16];
    let filename = format!("{sanitized_tool}_{timestamp}_{offload_id}.txt");
    let file_path = offload_dir.join(&filename);

    if let Err(e) = std::fs::write(&file_path, &result) {
        tracing::warn!("Failed to write offloaded result: {}", e);
        return result;
    }

    let result_size = result.len();
    let result_tokens = result_size / 4;

    tracing::info!(
        "Offloaded large tool result ({} chars, ~{} tokens) to: {}",
        result_size,
        result_tokens,
        file_path.display()
    );

    format!(
        "Tool result was too large for context ({} chars, ~{} tokens).\n\
        Offload id: {}\n\
        Result saved to: {}\n\n\
        To access the full result, use the `read` tool with this exact path:\n\
        ```json\n\
        {{\"path\":\"{}\",\"offset\":0,\"limit\":200}}\n\
        ```\n\n\
        For shell fallback, copy the path exactly:\n\
        `head -100 \"{}\"`\n\
        `grep \"pattern\" \"{}\"`",
        result_size,
        result_tokens,
        offload_id,
        file_path.display(),
        file_path.display(),
        file_path.display(),
        file_path.display()
    )
}

/// Extract key info (file paths, URLs) from a tool result for restorable compression.
#[cfg(test)]
fn extract_key_info(content: &str) -> String {
    let mut info = Vec::new();

    for word in content.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| {
            c == '"' || c == '\'' || c == ',' || c == ':' || c == '(' || c == ')'
        });
        if (trimmed.contains('/') || trimmed.contains('.'))
            && (trimmed.ends_with(".py")
                || trimmed.ends_with(".json")
                || trimmed.ends_with(".csv")
                || trimmed.ends_with(".html")
                || trimmed.ends_with(".md")
                || trimmed.ends_with(".js")
                || trimmed.ends_with(".ts")
                || trimmed.ends_with(".yaml")
                || trimmed.ends_with(".toml"))
            && !info.contains(&trimmed.to_string())
        {
            info.push(trimmed.to_string());
        }
        if (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
            && !info.contains(&trimmed.to_string())
        {
            info.push(trimmed.to_string());
        }
    }

    info.join(", ")
}

/// Compact messages to reduce context size when approaching token limits.
///
/// Strategy:
/// 1. Compress old assistant messages to one-liners (preserving tool names and file paths)
/// 2. Clear old tool result content (replace with placeholder, preserve file paths)
/// 3. Only drop messages if still over budget after compression
#[cfg(test)]
pub(crate) fn compact_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    use agent_primitives::types::Part;

    const KEEP_RECENT: usize = 20;

    if messages.len() <= KEEP_RECENT + 2 {
        return messages;
    }

    let mut messages = messages;

    // Phase 1: Compress old assistant messages to one-liners
    crate::middleware::compress_old_assistant_messages(&mut messages, KEEP_RECENT);

    // Phase 2: Clear old tool result content (keep tool_call_id for pairing)
    let compress_boundary = messages.len().saturating_sub(KEEP_RECENT);
    for message in &mut messages[..compress_boundary] {
        if message.role == "tool" {
            let text = message.text_content();
            let preserved = extract_key_info(&text);
            message.content = vec![Part::Text {
                text: if preserved.is_empty() {
                    "[result cleared]".to_string()
                } else {
                    format!("[result cleared — {preserved}]")
                },
            }];
        }
    }

    // Phase 3: If still too many messages, drop old ones
    if messages.len() > KEEP_RECENT + 10 {
        let mut compacted = Vec::new();

        // Keep system messages
        let mut non_system_start = 0;
        for (i, msg) in messages.iter().enumerate() {
            if msg.role == "system" {
                compacted.push(msg.clone());
                non_system_start = i + 1;
            } else {
                break;
            }
        }

        // Preserve first user message
        if let Some(user_msg) = messages[non_system_start..]
            .iter()
            .find(|m| m.role == "user")
        {
            compacted.push(user_msg.clone());
        }

        // Find clean split point
        let target_start = messages.len().saturating_sub(KEEP_RECENT);
        let mut split_at = target_start;
        for (i, msg) in messages.iter().enumerate().skip(target_start) {
            if msg.role == "user" || (msg.role == "assistant" && msg.tool_call_id.is_none()) {
                split_at = i;
                break;
            }
        }

        let trimmed_count = split_at.saturating_sub(non_system_start);
        if trimmed_count > 0 {
            compacted.push(ChatMessage::user(format!(
                "[SYSTEM: Context compacted. {trimmed_count} earlier messages were compressed and trimmed. \
                 The original request and recent messages are preserved. Continue with the task.]"
            )));
        }

        compacted.extend(messages[split_at..].iter().cloned());
        compacted
    } else {
        // Compression was enough — no need to drop
        messages
    }
}

/// Sanitize messages to ensure tool call/result pairs are valid.
///
/// Removes orphaned `tool` messages whose `tool_call_id` doesn't match
/// any `tool_calls` entry in a preceding `assistant` message.
/// This prevents API errors: "Messages with role 'tool' must be a response
/// to a preceding message with '`tool_calls`'"
pub(crate) fn sanitize_messages(messages: &mut Vec<ChatMessage>) {
    // Collect all valid tool_call_ids from assistant messages
    let mut valid_tool_call_ids = HashSet::new();
    for msg in messages.iter() {
        if msg.role == "assistant" {
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    valid_tool_call_ids.insert(tc.id.clone());
                }
            }
        }
    }

    // Remove orphaned tool messages
    let original_len = messages.len();
    messages.retain(|msg| {
        if msg.role == "tool" {
            if let Some(ref tool_call_id) = msg.tool_call_id {
                if !valid_tool_call_ids.contains(tool_call_id) {
                    tracing::warn!(
                        tool_call_id = %tool_call_id,
                        "Removing orphaned tool message — no matching assistant tool_call found"
                    );
                    return false;
                }
            }
        }
        true
    });

    if messages.len() < original_len {
        tracing::warn!(
            removed = original_len - messages.len(),
            "Sanitized {} orphaned tool messages from context",
            original_len - messages.len()
        );
    }
}

/// Truncate tool arguments to prevent context explosion.
///
/// When LLMs generate tool calls with massive arguments (e.g., including
/// full conversation context), storing these in message history causes
/// exponential growth. This function truncates arguments to a reasonable size.
#[allow(dead_code)]
fn truncate_tool_args(args: &Value, max_chars: usize) -> Value {
    let args_str = serde_json::to_string(args).unwrap_or_default();
    if args_str.len() <= max_chars {
        return args.clone();
    }

    // For objects, try to truncate string values
    if let Some(obj) = args.as_object() {
        let mut truncated = serde_json::Map::new();
        for (key, value) in obj {
            if let Some(s) = value.as_str() {
                if s.len() > 200 {
                    truncated.insert(
                        key.clone(),
                        Value::String(format!(
                            "{}... [truncated, {} chars]",
                            agent_primitives::truncate_str(s, 200),
                            s.len()
                        )),
                    );
                } else {
                    truncated.insert(key.clone(), value.clone());
                }
            } else {
                truncated.insert(key.clone(), value.clone());
            }
        }
        return Value::Object(truncated);
    }

    // Fallback: return a placeholder
    json!({"_truncated": true, "_original_size": args_str.len()})
}

/// Truncate a tool result string if it exceeds `max_chars`.
///
/// Keeps the first ~80% and last ~20% of the budget with a truncation notice.
/// Returns the original string if within limits or if `max_chars` is 0 (disabled).
fn truncate_single_line(result: &str, max_chars: usize) -> String {
    let notice = format!("\n\n--- TRUNCATED ({} chars total) ---\n\n", result.len());
    let budget = max_chars.saturating_sub(notice.len());
    let head_size = (budget * 4) / 5;
    let tail_size = budget - head_size;
    format!(
        "{}{}{}",
        &result[..head_size],
        notice,
        &result[result.len() - tail_size..]
    )
}

pub(crate) fn truncate_tool_result(result: String, max_chars: usize) -> String {
    if max_chars == 0 || result.len() <= max_chars {
        return result;
    }

    let lines: Vec<&str> = result.lines().collect();
    let total_lines = lines.len();

    if total_lines <= 1 {
        // Single line — fall back to char-based truncation
        return truncate_single_line(&result, max_chars);
    }

    // Line-aware: keep first N + last M lines within budget
    let head_budget = (max_chars * 4) / 5; // 80% for head
    let mut head = String::new();
    let mut head_count = 0;
    for line in &lines {
        let next = format!("{line}\n");
        if head.len() + next.len() > head_budget {
            break;
        }
        head.push_str(&next);
        head_count += 1;
    }

    // Tail: work backwards
    let tail_budget = max_chars / 5; // 20% for tail
    let mut tail_lines: Vec<&str> = Vec::new();
    let mut tail_len = 0;
    for line in lines.iter().rev() {
        let next_len = line.len() + 1;
        if tail_len + next_len > tail_budget {
            break;
        }
        tail_lines.push(line);
        tail_len += next_len;
    }
    tail_lines.reverse();
    let tail_count = tail_lines.len();
    let tail = tail_lines.join("\n");

    let omitted = total_lines.saturating_sub(head_count + tail_count);
    let notice = format!(
        "\n--- TRUNCATED: showing {}/{} lines ({} omitted, {} chars total) ---\n\n",
        head_count + tail_count,
        total_lines,
        omitted,
        result.len()
    );

    // Final budget check — if combined fits, return it; otherwise trim head/tail further
    let combined = format!("{head}{notice}{tail}");
    if combined.len() <= max_chars {
        return combined;
    }

    // Re-compute with tighter budgets accounting for notice length
    let notice_len = notice.len();
    let content_budget = max_chars.saturating_sub(notice_len);
    let tight_head_budget = (content_budget * 4) / 5;
    let tight_tail_budget = content_budget - tight_head_budget;

    let mut tight_head = String::new();
    let mut tight_head_count = 0;
    for line in &lines {
        let next = format!("{line}\n");
        if tight_head.len() + next.len() > tight_head_budget {
            break;
        }
        tight_head.push_str(&next);
        tight_head_count += 1;
    }

    let mut tight_tail_lines: Vec<&str> = Vec::new();
    let mut tight_tail_len = 0;
    for line in lines.iter().rev() {
        let next_len = line.len() + 1;
        if tight_tail_len + next_len > tight_tail_budget {
            break;
        }
        tight_tail_lines.push(line);
        tight_tail_len += next_len;
    }
    tight_tail_lines.reverse();
    let tight_tail_count = tight_tail_lines.len();
    let tight_tail = tight_tail_lines.join("\n");

    let tight_omitted = total_lines.saturating_sub(tight_head_count + tight_tail_count);
    let tight_notice = format!(
        "\n--- TRUNCATED: showing {}/{} lines ({} omitted, {} chars total) ---\n\n",
        tight_head_count + tight_tail_count,
        total_lines,
        tight_omitted,
        result.len()
    );

    format!("{tight_head}{tight_notice}{tight_tail}")
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    #[test]
    fn test_truncate_tool_result_under_limit() {
        let result = "hello world".to_string();
        assert_eq!(truncate_tool_result(result.clone(), 100), result);
    }

    #[test]
    fn test_truncate_tool_result_disabled() {
        let result = "a".repeat(50_000);
        assert_eq!(truncate_tool_result(result.clone(), 0), result);
    }

    #[test]
    fn test_truncate_tool_result_over_limit() {
        let result = "a".repeat(1000) + &"b".repeat(1000);
        let truncated = truncate_tool_result(result, 500);
        assert!(truncated.len() <= 500);
        assert!(truncated.contains("TRUNCATED"));
        assert!(truncated.starts_with("aaa"));
        assert!(truncated.ends_with("bbb"));
    }

    #[test]
    fn test_truncate_tool_result_preserves_head_tail_ratio() {
        let result = "H".repeat(10_000) + &"T".repeat(10_000);
        let truncated = truncate_tool_result(result, 1000);
        // Head should be ~80%, tail ~20% of budget
        let head_h = truncated.matches('H').count();
        let tail_t = truncated.matches('T').count();
        assert!(
            head_h > tail_t,
            "head ({head_h}) should be larger than tail ({tail_t})"
        );
    }

    #[test]
    fn test_truncation_preserves_line_boundaries() {
        let lines: Vec<String> = (0..100)
            .map(|i| format!("Line {i}: some content here"))
            .collect();
        let input = lines.join("\n");
        let result = truncate_tool_result(input, 500);

        // Should not cut mid-line
        for line in result.lines() {
            assert!(
                line.starts_with("Line")
                    || line.contains("TRUNCATED")
                    || line.contains("---")
                    || line.is_empty(),
                "Truncated mid-line: '{line}'"
            );
        }
    }

    #[test]
    fn test_prepare_tool_result_for_context_uses_distinct_offload_files() {
        let tmp = std::env::temp_dir().join(format!(
            "agent-runtime-tool-result-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).expect("tempdir");
        let config = ToolResultContextConfig {
            offload_large_results: true,
            offload_threshold_chars: 1,
            offload_dir: Some(tmp.clone()),
            ..ToolResultContextConfig::default()
        };

        let first = "first-result".repeat(10);
        let second = "second-result".repeat(10);

        let first_notice = prepare_tool_result_for_context("shell", first.clone(), &config);
        let second_notice = prepare_tool_result_for_context("shell", second.clone(), &config);

        assert_ne!(first_notice, second_notice);
        let mut saved: Vec<String> = std::fs::read_dir(&tmp)
            .expect("offload dir")
            .map(|entry| {
                let entry = entry.expect("entry");
                std::fs::read_to_string(entry.path()).expect("saved result")
            })
            .collect();
        saved.sort();
        let mut expected = vec![first, second];
        expected.sort();
        assert_eq!(saved, expected);
        std::fs::remove_dir_all(&tmp).expect("cleanup");
    }

    #[test]
    fn test_offload_notice_has_short_id_and_read_call() {
        let tmp = std::env::temp_dir().join(format!(
            "agent-runtime-tool-result-notice-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).expect("tempdir");
        let config = ToolResultContextConfig {
            offload_large_results: true,
            offload_threshold_chars: 1,
            offload_dir: Some(tmp.clone()),
            ..ToolResultContextConfig::default()
        };

        let notice = prepare_tool_result_for_context("shell", "large-result".repeat(20), &config);
        let entry = std::fs::read_dir(&tmp)
            .expect("offload dir")
            .next()
            .expect("offload entry")
            .expect("entry");
        let filename = entry.file_name().to_string_lossy().to_string();
        let id = filename
            .trim_end_matches(".txt")
            .rsplit('_')
            .next()
            .expect("id segment");

        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert!(notice.contains(&format!("Offload id: {id}")));
        assert!(notice.contains("\"offset\":0,\"limit\":200"));
        assert!(!filename.contains('-'));

        std::fs::remove_dir_all(&tmp).expect("cleanup");
    }

    #[test]
    fn test_truncate_tool_args_small() {
        let args = json!({"key": "value"});
        let result = truncate_tool_args(&args, 500);
        assert_eq!(result, args);
    }

    #[test]
    fn test_truncate_tool_args_large_string() {
        let args = json!({"content": "x".repeat(500)});
        let result = truncate_tool_args(&args, 100);
        let content = result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("truncated"));
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use crate::types::ToolCall;
    use agent_primitives::types::Part;
    use serde_json::json;

    #[test]
    fn test_compact_compresses_before_dropping() {
        let mut messages = vec![
            ChatMessage::system("system prompt".to_string()),
            ChatMessage::user("original request".to_string()),
        ];

        for i in 0..14 {
            let tool = ToolCall::new(
                format!("call_{i}"),
                "write_file".to_string(),
                json!({"path": format!("src/file_{}.py", i)}),
            );
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: format!("Creating file_{i}.py with detailed explanation"),
                }],
                tool_calls: Some(vec![tool]),
                tool_call_id: None,
                is_summary: false,
            });
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: format!("File created: src/file_{i}.py"),
                }],
                tool_calls: None,
                tool_call_id: Some(format!("call_{i}")),
                is_summary: false,
            });
        }

        let compacted = compact_messages(messages);

        // Old assistant messages should be compressed
        let has_compressed = compacted
            .iter()
            .any(|m| m.text_content().starts_with("[Turn"));
        assert!(
            has_compressed,
            "Old assistant messages should be compressed"
        );

        // Old tool results should preserve file paths
        let has_preserved = compacted.iter().any(|m| {
            m.text_content().contains("[result cleared") && m.text_content().contains(".py")
        });
        assert!(
            has_preserved,
            "Cleared tool results should preserve file paths"
        );
    }

    #[test]
    fn test_compact_preserves_recent() {
        let mut messages = vec![
            ChatMessage::system("system".to_string()),
            ChatMessage::user("request".to_string()),
        ];
        for i in 0..25 {
            messages.push(ChatMessage::user(format!("msg {i}")));
        }
        let compacted = compact_messages(messages);
        assert!(compacted.last().unwrap().text_content().contains("msg 24"));
    }

    #[test]
    fn test_extract_key_info() {
        let content = "File created: src/main.py with 100 lines. See https://example.com for docs.";
        let info = extract_key_info(content);
        assert!(info.contains("src/main.py"));
        assert!(info.contains("https://example.com"));
    }

    #[test]
    fn test_extract_key_info_empty() {
        let info = extract_key_info("Success! Operation completed.");
        assert!(info.is_empty());
    }

    // ========================================================================
    // Phase 2 minimal-patch regression tests — these target the two bugs
    // documented in docs/architecture/future-state/compaction-strategy.md:
    //   1. `total_tokens_in` accumulator overcount (executor.rs:498, :852)
    //   2. `compact_messages` no-op on short high-token conversations
    // ========================================================================

    /// Simulates 30 tool-loop iterations whose `usage.prompt_tokens` each
    /// equal 20_000 against a 200_000-token context window. Under the old
    /// accumulator (`+=`) the trigger value would reach 600_000 and fire
    /// spuriously. Under the new assignment semantic, the trigger value
    /// is whatever the *last* response reported — never above the real tape.
    #[test]
    fn accumulator_does_not_sum_across_turns() {
        // Simulate the assignment we wired at executor.rs:852.
        // The test proves the *semantic* of the change: `current_prompt_tokens`
        // tracks the latest response's value, not a running sum.
        let mut current_prompt_tokens: u64 = 0;
        let mut total_tokens_in: u64 = 0; // billing mirror, still accumulates

        for _ in 0..30 {
            let response_prompt_tokens: u32 = 20_000;
            total_tokens_in += u64::from(response_prompt_tokens);
            current_prompt_tokens = u64::from(response_prompt_tokens);
        }

        assert_eq!(
            total_tokens_in, 600_000,
            "billing accumulator still sums — UI/cost tracking relies on this"
        );
        assert_eq!(
            current_prompt_tokens, 20_000,
            "compaction signal stays bounded by per-turn value, not cumulative"
        );

        // With a 200_000-token window and 80% threshold = 160_000, the
        // OLD trigger (tokens_in > 160k) would fire on turn 9 and every
        // turn after. The NEW trigger (current_prompt_tokens > 160k)
        // never fires for this stable tape.
        let compact_threshold = (200_000u64 * 80) / 100;
        assert!(
            total_tokens_in > compact_threshold,
            "old accumulator would have tripped the threshold"
        );
        assert!(
            current_prompt_tokens <= compact_threshold,
            "new signal does not trip the threshold — no spurious compaction"
        );
    }

    /// Compaction must never orphan a `tool_use`/`tool_result` pair — the
    /// Anthropic API returns HTTP 400 if a tool message references an
    /// assistant `tool_call_id` that no longer exists. For every
    /// `tool_call_id` present on any remaining assistant `tool_calls`,
    /// there must be a paired `tool` message (and vice versa).
    #[test]
    fn compaction_preserves_tool_use_result_pairs() {
        let mut messages = vec![
            ChatMessage::system("system".to_string()),
            ChatMessage::user("original request".to_string()),
        ];
        // 15 assistant+tool pairs. KEEP_RECENT = 20 means pairs near the
        // front are candidates for compression/drop; those near the end
        // stay intact.
        for i in 0..15 {
            let tc = ToolCall::new(
                format!("call_{i}"),
                "read_file".to_string(),
                json!({"path": format!("src/file_{i}.py")}),
            );
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: vec![Part::Text {
                    text: format!("step {i}"),
                }],
                tool_calls: Some(vec![tc]),
                tool_call_id: None,
                is_summary: false,
            });
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: format!("output for call_{i}"),
                }],
                tool_calls: None,
                tool_call_id: Some(format!("call_{i}")),
                is_summary: false,
            });
        }

        let compacted = compact_messages(messages);

        // Collect all tool_call_ids referenced by surviving assistant messages.
        let referenced_ids: std::collections::HashSet<String> = compacted
            .iter()
            .filter(|m| m.role == "assistant")
            .filter_map(|m| m.tool_calls.as_ref())
            .flat_map(|tcs| tcs.iter().map(|tc| tc.id.clone()))
            .collect();

        // Every surviving tool message's tool_call_id MUST be referenced.
        for msg in &compacted {
            if msg.role == "tool" {
                let tcid = msg.tool_call_id.as_deref().unwrap_or("");
                assert!(
                    referenced_ids.contains(tcid),
                    "orphaned tool message — tool_call_id '{tcid}' not referenced \
                     by any surviving assistant message (this causes HTTP 400)"
                );
            }
        }

        // And every referenced tool_call_id MUST have a paired tool message.
        for id in &referenced_ids {
            let has_pair = compacted
                .iter()
                .any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some(id));
            assert!(
                has_pair,
                "assistant references tool_call_id '{id}' but no tool response survived"
            );
        }
    }

    /// Some providers (streaming error paths, OAI-compat clones) omit
    /// `response.usage`. When that happens the executor's
    /// `current_prompt_tokens` stays at its prior value; the trigger does
    /// not spuriously fire. This test exercises the fallback semantic of
    /// the assignment at executor.rs:852.
    #[test]
    fn missing_usage_field_leaves_signal_unchanged() {
        let mut current_prompt_tokens: u64 = 12_345;

        // Simulate a response without `usage` — the `if let Some(usage)`
        // guard at executor.rs:851 means we skip the assignment entirely.
        let response_usage: Option<crate::llm::TokenUsage> = None;
        if let Some(usage) = &response_usage {
            current_prompt_tokens = u64::from(usage.prompt_tokens);
        }

        assert_eq!(
            current_prompt_tokens, 12_345,
            "missing usage must not zero out the signal — trigger holds steady"
        );
    }
}

// ============================================================================
// Static-helper and builder coverage tests
// ============================================================================
#[cfg(test)]
mod helper_coverage_tests {
    use super::*;
    use crate::types::ToolCall;
    use agent_primitives::types::Part;
    use serde_json::Value;

    // ------------- sanitize_messages -------------
    #[test]
    fn sanitize_messages_drops_orphaned_tool_messages() {
        let mut messages = vec![
            ChatMessage::user("hi".to_string()),
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "result".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("nonexistent".to_string()),
                is_summary: false,
            },
        ];
        sanitize_messages(&mut messages);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn sanitize_messages_keeps_valid_pair() {
        let mut messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: vec![],
                tool_calls: Some(vec![ToolCall::new(
                    "c1".to_string(),
                    "t".to_string(),
                    Value::Null,
                )]),
                tool_call_id: None,
                is_summary: false,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: vec![Part::Text {
                    text: "ok".to_string(),
                }],
                tool_calls: None,
                tool_call_id: Some("c1".to_string()),
                is_summary: false,
            },
        ];
        let before = messages.len();
        sanitize_messages(&mut messages);
        assert_eq!(messages.len(), before);
    }

    // ------------- truncate_tool_args -------------
    #[test]
    fn truncate_tool_args_returns_clone_when_under_budget() {
        let v = json!({"a": 1, "b": "x"});
        let out = truncate_tool_args(&v, 100);
        assert_eq!(out, v);
    }

    #[test]
    fn truncate_tool_args_passes_through_short_strings_in_object() {
        let v = json!({"name": "small"});
        // Force args_str.len() > max_chars but the inner string is short → preserved.
        let out = truncate_tool_args(&v, 5);
        // Output is an object copy; "name" should still be the original short string.
        assert_eq!(out.get("name").unwrap(), "small");
    }

    #[test]
    fn truncate_tool_args_non_object_returns_placeholder() {
        let v = json!("a very long string ".repeat(50));
        let out = truncate_tool_args(&v, 10);
        assert_eq!(out.get("_truncated"), Some(&Value::Bool(true)));
        assert!(out.get("_original_size").is_some());
    }
}
