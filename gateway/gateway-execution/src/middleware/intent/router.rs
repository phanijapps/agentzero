//! Intent router — classify a request into an orchestration decision.
//!
//! Three paths, cheapest first:
//! 1. Trivial messages (greetings) — no LLM, default analysis.
//! 2. Deterministic procedure name match — a request naming a learned
//!    procedure is a macro invocation: route `simple`, pin the procedure.
//!    The global name index is deliberately NOT ward-scoped; wards organize
//!    context, not callables.
//! 3. One structured LLM call over retrieved candidates — one retry on
//!    transient provider failure, then the semantic-ward default.

use super::contract::{
    ExecutionApproach, ExecutionStrategy, IntentAnalysis, PinnedProcedure, WardAction,
    WardRecommendation,
};
use super::prompt::format_user_template;
use crate::middleware::resource_index::search_resources;
use agent_runtime::{ContextActorKind, LlmClient};
use agent_tools::{GoalAccess, RecallAuthorizationContext};
use serde_json::Value;
use zbot_stores::MemoryFactStore;
use zbot_stores_traits::ProcedureStore;

/// Reason marker on the trivial-message placeholder analysis. The fast-path
/// ward note must not render for it: no classifier ran, so "domain matches
/// the `general` ward" would be a fabricated claim on the greeting path.
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
        .any(|pattern| lower == *pattern || (lower.starts_with(pattern) && word_count <= 4))
}

/// Default "simple" analysis for trivial messages and degraded paths.
pub(crate) fn simple_analysis(message: &str) -> IntentAnalysis {
    IntentAnalysis {
        primary_intent: message.chars().take(100).collect(),
        hidden_intents: vec![],
        recommended_skills: vec![],
        recommended_agents: vec![],
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
        pinned_procedure: None,
    }
}

/// Degraded default: warm-route via the top semantic ward match so a
/// provider failure costs one classification, not routing itself.
fn fallback_analysis_from_semantic(wards: &[String]) -> IntentAnalysis {
    let mut analysis = simple_analysis("");
    analysis.primary_intent = String::new();
    match wards.first() {
        Some(top_ward) => {
            analysis.ward_recommendation = WardRecommendation {
                action: WardAction::UseExisting,
                ward_name: top_ward.clone(),
                subdirectory: None,
                structure: std::collections::HashMap::new(),
                reason: "Classifier unavailable — warm-routing via top semantic ward match"
                    .to_string(),
            };
        }
        None => {
            analysis.ward_recommendation.action = WardAction::CreateNew;
            analysis.ward_recommendation.reason =
                "Classifier unavailable and no ward matched — create_new".to_string();
        }
    }
    analysis
}

/// Deterministic global procedure match: a request that names a learned
/// procedure is a macro invocation — no classifier call needed. Matching is
/// containment of the exact procedure name (word-boundary, case-insensitive)
/// so short generic names don't false-positive inside longer words.
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
        .find(|(name, _ward_id)| {
            let needle = name.to_lowercase();
            needle.len() >= 4
                && haystack
                    .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
                    .any(|word| word == needle)
        })
        .map(|(name, ward_id)| PinnedProcedure { name, ward_id })
}

/// Classify one user request. Never fails: the worst case is the semantic
/// fallback with warm ward routing.
#[allow(clippy::too_many_arguments)]
pub async fn analyze_intent(
    llm_client: std::sync::Arc<dyn LlmClient>,
    user_message: &str,
    fact_store: &dyn MemoryFactStore,
    memory_recall: Option<&std::sync::Arc<crate::recall::MemoryRecall>>,
    goal_access: Option<std::sync::Arc<dyn GoalAccess>>,
    recall_authorization: Option<RecallAuthorizationContext>,
    system_prompt: &str,
    procedure_store: Option<&dyn ProcedureStore>,
    existing_wards: &[String],
    available_mcps: &[Value],
) -> IntentAnalysis {
    if is_simple_message(user_message) {
        tracing::info!("Intent router — trivial message, no classifier call");
        return simple_analysis(user_message);
    }

    // Macro invocations skip the classifier entirely: deterministic match,
    // simple posture, pinned procedure with its home ward.
    if let Some(pinned) = match_procedure_by_name(procedure_store, user_message).await {
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

    // Retrieved memory context sharpens the classifier's judgment (scoped,
    // sanitized by the unified recall policy).
    let memory_context =
        if let (Some(recall), Some(authorization)) = (memory_recall, recall_authorization) {
            match crate::invoke::unified_recall_adapter::automatic_unified_recall(
                recall.clone(),
                goal_access,
                authorization,
                user_message,
                10,
            )
            .await
            {
                Ok(response) if !response.results.is_empty() => {
                    crate::recall::format_unified_recall_response_with_options(
                        &response,
                        crate::recall::ContextPacketBuildOptions::new(
                            "intent-analysis-recall",
                            "root",
                            ContextActorKind::Root,
                            900,
                        ),
                    )
                }
                Ok(_) => String::new(),
                Err(e) => {
                    tracing::warn!(reason = ?e.code, "Intent-analysis unified recall failed");
                    String::new()
                }
            }
        } else {
            String::new()
        };

    let results = search_resources(fact_store, user_message).await;
    tracing::info!(
        skills = results.skills.len(),
        agents = results.agents.len(),
        wards = results.wards.len(),
        mcps = results.mcps.len(),
        "Retrieved resource candidates"
    );

    // MCP candidates intersect with the runtime-safe catalog so stale facts
    // can never surface a deleted/disabled server.
    let available_mcp_ids = available_mcps
        .iter()
        .filter_map(|mcp| mcp.get("id").and_then(Value::as_str))
        .collect::<std::collections::HashSet<_>>();
    let relevant_mcps = results
        .mcps
        .iter()
        .filter(|mcp| {
            mcp.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| available_mcp_ids.contains(id))
        })
        .cloned()
        .collect::<Vec<_>>();
    let mcp_prompt_candidates = if relevant_mcps.is_empty() {
        available_mcps.iter().take(8).cloned().collect::<Vec<_>>()
    } else {
        relevant_mcps
    };

    let user_template = format_user_template(
        user_message,
        &results.skills,
        &results.agents,
        &mcp_prompt_candidates,
        existing_wards,
    );
    let user_content = if memory_context.is_empty() {
        user_template
    } else {
        format!("{}\n\n{}", memory_context, user_template)
    };

    // One structured call; one retry for transient provider failures; then
    // the semantic-ward default. A valid strict schema (see the wire
    // normalization in agent-runtime) makes the retry genuinely transient
    // cover rather than a second spin of a broken request.
    let mut analysis = match agent_runtime::rig_adapter::prompt_typed::<IntentAnalysis>(
        llm_client.clone(),
        system_prompt,
        user_content.as_str(),
    )
    .await
    {
        Ok(analysis) => analysis,
        Err(first_err) => {
            tracing::warn!(
                error = %first_err,
                "Intent classifier failed on first attempt — retrying once"
            );
            match agent_runtime::rig_adapter::prompt_typed::<IntentAnalysis>(
                llm_client,
                system_prompt,
                user_content.as_str(),
            )
            .await
            {
                Ok(analysis) => analysis,
                Err(second_err) => {
                    tracing::warn!(
                        error = %second_err,
                        wards = ?results.wards,
                        "Intent classifier failed twice — semantic ward fallback"
                    );
                    return fallback_analysis_from_semantic(&results.wards);
                }
            }
        }
    };

    analysis.pinned_procedure = None;

    // Deterministic override: the classifier (a thinking model reading a
    // long rubric) under-routes research prompts to simple. If either the
    // user's message or the classified intent contains explicit depth
    // signals, force graph — the rubric already says these ARE graph; this
    // just enforces what the LLM reads but doesn't follow.
    const DEPTH_SIGNALS: &[&str] = &[
        "comprehensive",
        "in-depth",
        "in depth",
        "deep analysis",
        "deep research",
        "critical analysis",
        "comparative analysis",
        "literature review",
        "research report",
        "multi-source",
        "rigorous",
        "extensive",
        "thorough",
        "detailed analysis",
    ];
    let haystack = format!("{} {}", user_message, analysis.primary_intent).to_lowercase();
    let has_depth_signal = DEPTH_SIGNALS.iter().any(|s| haystack.contains(s));
    if has_depth_signal && analysis.execution_strategy.approach == ExecutionApproach::Simple {
        tracing::info!(
            primary_intent = %analysis.primary_intent,
            "Depth-signal override: research prompt classified simple → graph"
        );
        analysis.execution_strategy.approach = ExecutionApproach::Graph;
        if analysis.execution_strategy.explanation.is_empty() {
            analysis.execution_strategy.explanation =
                "Research-depth signals in the request require orchestrated execution".to_string();
        }
    }

    tracing::info!(
        primary_intent = %analysis.primary_intent,
        ward = %analysis.ward_recommendation.ward_name,
        approach = %analysis.execution_strategy.approach,
        "Intent classification complete"
    );
    analysis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_messages_skip_everything() {
        assert!(is_simple_message("hello"));
        assert!(is_simple_message("  Hey there!  "));
        assert!(is_simple_message("thanks a lot"));
        assert!(!is_simple_message("hello, please analyze my portfolio"));
        assert!(!is_simple_message(
            "Run the peer_valuation_pipeline procedure on AAPL vs MSFT"
        ));
    }

    #[test]
    fn fallback_warm_routes_to_top_semantic_ward() {
        let a = fallback_analysis_from_semantic(&["history-library".into(), "other".into()]);
        assert_eq!(a.ward_recommendation.ward_name, "history-library");
        assert_eq!(a.ward_recommendation.action, WardAction::UseExisting);
        let b = fallback_analysis_from_semantic(&[]);
        assert_eq!(b.ward_recommendation.action, WardAction::CreateNew);
    }

    struct NameStore;
    #[async_trait::async_trait]
    impl ProcedureStore for NameStore {
        async fn list_procedure_names(
            &self,
            _agent_id: &str,
            _limit: usize,
        ) -> Result<Vec<(String, Option<String>)>, String> {
            Ok(vec![
                (
                    "peer_valuation_pipeline".into(),
                    Some("financial-analysis".into()),
                ),
                ("do".into(), None), // too short — must never match
            ])
        }
    }

    #[tokio::test]
    async fn procedure_match_is_word_boundary_and_cross_ward() {
        let store = NameStore;
        let hit =
            match_procedure_by_name(Some(&store), "Please run peer_valuation_pipeline for AAPL")
                .await
                .expect("name match");
        assert_eq!(hit.name, "peer_valuation_pipeline");
        assert_eq!(hit.ward_id.as_deref(), Some("financial-analysis"));

        // Substring containment is NOT a match ("peer_valuation_pipelines" is a
        // different token), and short names never fire.
        assert!(
            match_procedure_by_name(Some(&store), "peer_valuation_pipelines please")
                .await
                .is_none()
        );
        assert!(match_procedure_by_name(Some(&store), "just do it")
            .await
            .is_none());
        assert!(match_procedure_by_name(None, "run peer_valuation_pipeline")
            .await
            .is_none());
    }
}
