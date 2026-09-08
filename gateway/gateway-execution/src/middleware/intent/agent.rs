//! The intent agent: one call, tools + schema-enforced output.
//! The model searches the index via MemoryTool, returns IntentAnalysis.

use super::contract::IntentAnalysis;
use agent_runtime::rig_adapter::RigToolAdapter;
use agent_tools::MemoryTool;
use gateway_services::providers::Provider;
use gateway_services::SharedVaultPaths;
use std::sync::Arc;
use zbot_stores::MemoryFactStore;
use zbot_stores_traits::ProcedureStore;

pub struct IntentAgentDeps {
    pub fact_store: Arc<dyn MemoryFactStore>,
    pub procedure_store: Option<Arc<dyn ProcedureStore>>,
    pub paths: SharedVaultPaths,
    pub provider: Provider,
    pub model: String,
    pub max_tokens: u64,
}

/// Run the intent agent: single call with MemoryTool + IntentAnalysis schema.
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

    let memory_tool = MemoryTool::new(
        Arc::new(crate::config::GatewayFileSystem::new(
            deps.paths.vault_dir().clone(),
        )),
        Some(deps.fact_store.clone()),
    );

    match agent_runtime::rig_adapter::agent_with_tools_and_schema::<IntentAnalysis>(
        client,
        deps.model.clone(),
        super::prompt::INTENT_AGENT_PROMPT,
        vec![RigToolAdapter::boxed(Arc::new(memory_tool))],
        message,
    )
    .await
    {
        Ok(analysis) => {
            tracing::info!(
                primary_intent = %analysis.primary_intent,
                approach = %analysis.execution_strategy.approach,
                "Intent agent complete"
            );
            Some(analysis)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Intent agent failed");
            None
        }
    }
}
