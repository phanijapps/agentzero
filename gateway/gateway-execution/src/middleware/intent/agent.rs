//! The intent agent: the model searches via MemorySearchTool, reasons, outputs JSON.
//! The agent decides simple vs graph — no pre-checks, no pre-fetched results.

use super::contract::IntentAnalysis;
use agent_primitives::vault_paths::SharedVaultPaths;
use agent_runtime::rig_adapter::RigToolAdapter;
use agent_tools::MemorySearchTool;
use gateway_services::providers::Provider;
use std::sync::Arc;
use zbot_stores_traits::MemoryFactStore;
use zbot_stores_traits::ProcedureStore;

pub struct IntentAgentDeps {
    pub fact_store: Arc<dyn MemoryFactStore>,
    pub procedure_store: Option<Arc<dyn ProcedureStore>>,
    pub paths: SharedVaultPaths,
    pub provider: Provider,
    pub model: String,
    pub max_tokens: u64,
}

/// Run the intent agent: MemorySearchTool + prompt → JSON.
/// The model calls search_memory as many times as it needs,
/// then outputs its analysis as JSON.
pub async fn run_intent_agent(deps: &IntentAgentDeps, message: &str) -> Option<IntentAnalysis> {
    let llm_config = agent_runtime::LlmConfig::new(
        deps.provider.base_url.clone(),
        deps.provider.api_key.clone(),
        deps.model.clone(),
        deps.provider
            .id
            .clone()
            .unwrap_or_else(|| deps.provider.name.clone()),
    )
    .with_max_tokens(deps.max_tokens as u32);
    let client: Arc<dyn agent_runtime::llm::LlmClient> =
        Arc::new(agent_runtime::OpenAiClient::new(llm_config).ok()?);

    // Agent with MemorySearchTool — the model drives the search
    let result = agent_runtime::rig_adapter::agent_with_tools(
        client,
        deps.model.clone(),
        super::prompt::INTENT_AGENT_PROMPT,
        vec![RigToolAdapter::boxed(Arc::new(MemorySearchTool::new(
            deps.fact_store.clone(),
        )))],
        message,
    )
    .await;

    match result {
        Ok(text) => {
            let content = text.trim();
            // Model returns JSON after searching. An analysis whose
            // primary_intent parses to empty is NOT a success — serde
            // accepts "" and downstream silently degrades (observed:
            // sess-b9ed1722, 25s of agent work logged "succeeded" with
            // an empty intent and no Task Analysis injection). Treat it
            // as a parse failure: try the fence-extract, then None.
            let parsed = serde_json::from_str::<IntentAnalysis>(content)
                .ok()
                .or_else(|| {
                    let start = content.find('{')?;
                    let end = content.rfind('}')?;
                    serde_json::from_str::<IntentAnalysis>(&content[start..=end]).ok()
                });
            match parsed {
                Some(analysis) if !analysis.primary_intent.trim().is_empty() => {
                    tracing::info!(
                        primary_intent = %analysis.primary_intent,
                        approach = %analysis.execution_strategy.approach,
                        "Intent agent complete"
                    );
                    Some(analysis)
                }
                Some(_analysis) => {
                    tracing::warn!(
                        raw_chars = content.chars().count(),
                        "Intent agent returned an empty primary_intent — treating as failure"
                    );
                    None
                }
                None => None,
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Intent agent failed");
            None
        }
    }
}

#[cfg(test)]
mod empty_intent_tests {
    use super::super::contract::IntentAnalysis;

    #[test]
    fn empty_primary_intent_is_detectable_at_parse_boundary() {
        // The guard lives in run_intent_agent's match, but the contract
        // shape it protects is asserted here: serde happily accepts an
        // empty intent, which is why the explicit emptiness check exists.
        let raw = r#"{"primary_intent":"","hidden_intents":[],"solution_path":[],"recommended_skills":[],"recommended_agents":[],"recommended_procedures":[],"recommended_capabilities":[],"ward_recommendation":{"action":"use_existing","ward_name":"general","subdirectory":null,"structure":{},"reason":""},"execution_strategy":{"approach":"simple","explanation":""},"complexity":null,"explanation":""}"#;
        let analysis: IntentAnalysis = serde_json::from_str(raw).expect("parses");
        assert!(analysis.primary_intent.trim().is_empty());
    }
}
