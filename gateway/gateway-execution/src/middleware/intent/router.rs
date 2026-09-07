//! Intent routing: the intent agent searches the index and submits an analysis.

use super::agent::{run_intent_agent, IntentAgentDeps};
use super::contract::{
    ExecutionApproach, ExecutionStrategy, IntentAnalysis, PinnedProcedure, WardAction,
    WardRecommendation,
};
use gateway_services::SharedVaultPaths;
use zbot_stores_traits::ProcedureStore;

pub(crate) const TRIVIAL_WARD_REASON: &str = "Simple request — no ward needed";

pub(crate) fn is_simple_message(message: &str) -> bool {
    let trimmed = message.trim();
    let word_count = trimmed.split_whitespace().count();
    let simple_patterns = [
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
    simple_patterns
        .iter()
        .any(|p| lower == *p || (lower.starts_with(p) && word_count <= 4))
}

pub(crate) fn simple_analysis(message: &str) -> IntentAnalysis {
    IntentAnalysis {
        primary_intent: message.chars().take(100).collect(),
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
            reason: TRIVIAL_WARD_REASON.to_string(),
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

/// Deterministic global procedure match.
async fn match_procedure_by_name(
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

/// List wards on disk as (name, purpose) pairs — used by the fallback.
fn list_wards(paths: &SharedVaultPaths) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(paths.wards_dir()) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let purpose = std::fs::read_to_string(entry.path().join("AGENTS.md"))
                .ok()
                .and_then(|content| {
                    content
                        .lines()
                        .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                        .map(|l| l.trim().to_string())
                })
                .unwrap_or_default();
            (name, purpose)
        })
        .collect()
}

fn fallback_analysis(wards: &[(String, String)]) -> IntentAnalysis {
    let mut analysis = simple_analysis("");
    analysis.primary_intent = String::new();
    match wards.first() {
        Some((top_ward, _)) => {
            analysis.ward_recommendation = WardRecommendation {
                action: WardAction::UseExisting,
                ward_name: top_ward.clone(),
                subdirectory: None,
                structure: std::collections::HashMap::new(),
                reason: "Intent agent unavailable — warm-routing via top ward".to_string(),
            };
        }
        None => {
            analysis.ward_recommendation.action = WardAction::CreateNew;
            analysis.ward_recommendation.reason =
                "Intent agent unavailable and no ward matched — create_new".to_string();
        }
    }
    analysis
}

/// Classify one user request. Never fails — the worst case is the fallback.
pub async fn analyze_intent(deps: &IntentAgentDeps, user_message: &str) -> IntentAnalysis {
    if is_simple_message(user_message) {
        tracing::info!("Intent router — trivial message, no agent call");
        return simple_analysis(user_message);
    }

    if let Some(pinned) =
        match_procedure_by_name(deps.procedure_store.as_deref(), user_message).await
    {
        tracing::info!(
            procedure = %pinned.name,
            ward = ?pinned.ward_id,
            "Intent router — deterministic procedure match"
        );
        let mut analysis = simple_analysis(user_message);
        analysis.execution_strategy.explanation =
            "Request names a learned procedure — direct invocation".to_string();
        analysis.pinned_procedure = Some(pinned);
        return analysis;
    }

    tracing::info!("Intent router — running intent agent");
    let wards = list_wards(&deps.paths);

    match run_intent_agent(deps, user_message).await {
        Some(mut analysis) => {
            analysis.pinned_procedure = None;
            tracing::info!(
                primary_intent = %analysis.primary_intent,
                approach = %analysis.execution_strategy.approach,
                complexity = ?analysis.complexity,
                "Intent agent complete"
            );
            analysis
        }
        None => {
            tracing::warn!("Intent agent did not submit — falling back");
            fallback_analysis(&wards)
        }
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
