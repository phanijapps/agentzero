//! Intent routing: trivial bypass + procedure match + agent (which searches).

use super::agent::{run_intent_agent, IntentAgentDeps};
use super::contract::{
    ExecutionApproach, ExecutionStrategy, IntentAnalysis, PinnedProcedure, WardAction,
    WardRecommendation,
};
use agent_primitives::vault_paths::SharedVaultPaths;
use zbot_stores_traits::ProcedureStore;

/// Greetings and non-task messages bypass the agent entirely — no LLM call.
fn is_trivial(message: &str) -> bool {
    let trimmed = message.trim();
    let word_count = trimmed.split_whitespace().count();
    let trivial = [
        "hello",
        "hi",
        "hey",
        "good morning",
        "good afternoon",
        "good evening",
        "thanks",
        "thank you",
        "bye",
        "goodbye",
        "what's up",
        "how are you",
        "help",
        "what can you do",
        "who are you",
    ];
    let lower = trimmed.to_lowercase();
    trivial
        .iter()
        .any(|p| lower == *p || (lower.starts_with(p) && word_count <= 4))
}

fn trivial_analysis() -> IntentAnalysis {
    IntentAnalysis {
        primary_intent: String::new(),
        hidden_intents: vec![],
        solution_path: vec![],
        recommended_skills: vec![],
        recommended_agents: vec![],
        recommended_procedures: vec![],
        recommended_capabilities: vec![],
        ward_recommendation: WardRecommendation {
            action: WardAction::UseExisting,
            ward_name: "general".to_string(),
            subdirectory: None,
            structure: std::collections::HashMap::new(),
            reason: "Trivial message".to_string(),
        },
        execution_strategy: ExecutionStrategy {
            approach: ExecutionApproach::Simple,
            explanation: String::new(),
        },
        complexity: None,
        explanation: String::new(),
        pinned_procedure: None,
    }
}

/// Deterministic procedure name match — no LLM needed.
async fn match_procedure(
    procedure_store: Option<&dyn ProcedureStore>,
    message: &str,
) -> Option<PinnedProcedure> {
    let store = procedure_store?;
    let names = store
        .list_procedure_names("root", 500)
        .await
        .unwrap_or_default();
    if names.is_empty() {
        return None;
    }
    let haystack = message.to_lowercase();
    names
        .into_iter()
        .find(|(name, _)| {
            let needle = name.to_lowercase();
            needle.len() >= 4
                && haystack
                    .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
                    .any(|word| word == needle)
        })
        .map(|(name, ward_id)| PinnedProcedure { name, ward_id })
}

/// Labeled fallback for a failed/empty intent-agent result.
///
/// The previous fallback returned trivial_analysis() verbatim: an EMPTY
/// primary_intent logged downstream as "Intent analysis succeeded", and a
/// ward recommendation of "general" that bootstrap's filesystem ground
/// truth could "correct" into CREATE — active misdirection for a path that
/// knows nothing (observed: sess-66582eff). The labeled fallback seeds the
/// intent from the message head and marks the explanation so logs and the
/// injection surface can distinguish it from a real analysis.
fn fallback(user_message: &str) -> IntentAnalysis {
    let mut analysis = trivial_analysis();
    let seed: String = user_message
        .trim()
        .chars()
        .take(60)
        .collect::<String>()
        .lines()
        .next()
        .unwrap_or("user request")
        .to_string();
    analysis.primary_intent = seed;
    analysis.execution_strategy.explanation =
        "intent agent unavailable or returned empty — fallback analysis".to_string();
    analysis
}

/// Classify a user request. The agent searches and decides simple vs graph.
pub async fn analyze_intent(deps: &IntentAgentDeps, user_message: &str) -> IntentAnalysis {
    if is_trivial(user_message) {
        return trivial_analysis();
    }

    if let Some(pinned) = match_procedure(deps.procedure_store.as_deref(), user_message).await {
        let mut analysis = trivial_analysis();
        analysis.execution_strategy.explanation =
            "Request names a learned procedure — direct invocation".to_string();
        analysis.pinned_procedure = Some(pinned);
        return analysis;
    }

    match run_intent_agent(deps, user_message).await {
        Some(analysis) => analysis,
        None => fallback(user_message),
    }
}

/// Load the intent prompt, preferring the vault-local override.
pub fn load_intent_analysis_prompt(paths: &SharedVaultPaths) -> String {
    let override_path = paths.config_dir().join("intent-analysis-prompt.md");
    match std::fs::read_to_string(&override_path) {
        Ok(content) if !content.trim().is_empty() => content,
        _ => super::prompt::INTENT_AGENT_PROMPT.to_string(),
    }
}
