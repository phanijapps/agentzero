//! The intent agent: Rig engine + system prompt + tools → structured response.
//!
//! Uses the existing MemoryTool (semantic search over indexed resources)
//! and one SubmitIntentTool (structured output). Nothing custom.

use super::contract::IntentAnalysis;
use agent_primitives::error::AgentError;
use agent_primitives::{Tool, ToolContext as ToolContextTrait};
use agent_runtime::llm::LlmClient;
use agent_runtime::rig_adapter::model::LlmCompletionModel;
use agent_runtime::rig_adapter::RigAgentConfig;
use agent_runtime::rig_adapter::RigModelConfig;
use agent_runtime::{AgentEngine, ToolContext};
use agent_tools::MemoryTool;
use gateway_services::providers::Provider;
use gateway_services::SharedVaultPaths;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbot_stores::MemoryFactStore;
use zbot_stores_traits::ProcedureStore;

// ---------------------------------------------------------------------------
// Submit tool — structured output via tool call
// ---------------------------------------------------------------------------

pub struct SubmitIntentTool {
    result: Arc<Mutex<Option<IntentAnalysis>>>,
}

impl SubmitIntentTool {
    pub fn new() -> (Self, Arc<Mutex<Option<IntentAnalysis>>>) {
        let result = Arc::new(Mutex::new(None));
        (
            Self {
                result: result.clone(),
            },
            result,
        )
    }
}

#[async_trait::async_trait]
impl Tool for SubmitIntentTool {
    fn name(&self) -> &'static str {
        "submit_intent"
    }
    fn description(&self) -> &'static str {
        "Submit your intent analysis. This is the ONLY way to complete."
    }
    fn parameters_schema(&self) -> Option<Value> {
        serde_json::to_value(schemars::schema_for!(IntentAnalysis)).ok()
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        args: Value,
    ) -> agent_primitives::error::Result<Value> {
        let analysis: IntentAnalysis = serde_json::from_value(args)
            .map_err(|e| AgentError::Tool(format!("invalid intent analysis: {e}")))?;
        *self.result.lock().unwrap() = Some(analysis);
        Ok(serde_json::json!({"status": "submitted"}))
    }
}

// ---------------------------------------------------------------------------
// Intent agent — build, execute, extract
// ---------------------------------------------------------------------------

pub struct IntentAgentDeps {
    pub fact_store: Arc<dyn MemoryFactStore>,
    pub procedure_store: Option<Arc<dyn ProcedureStore>>,
    pub paths: SharedVaultPaths,
    pub provider: Provider,
    pub model: String,
    pub max_tokens: u64,
}

/// Run the intent agent: system prompt + memory tool + submit tool.
/// The model reasons, searches via memory tool, submits via submit_intent.
pub async fn run_intent_agent(deps: &IntentAgentDeps, message: &str) -> Option<IntentAnalysis> {
    // LLM client from the configured intent model
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
    let client: Arc<dyn LlmClient> = Arc::new(agent_runtime::OpenAiClient::new(llm_config).ok()?);
    let model = LlmCompletionModel::new(client, deps.model.clone());

    // Tools: existing MemoryTool + SubmitIntentTool
    let (submit, result_slot) = SubmitIntentTool::new();
    let tools = vec![
        agent_runtime::rig_adapter::RigToolAdapter::boxed(Arc::new(MemoryTool::new(
            Arc::new(crate::config::GatewayFileSystem::new(
                deps.paths.vault_dir().clone(),
            )),
            Some(deps.fact_store.clone()),
        ))),
        agent_runtime::rig_adapter::RigToolAdapter::boxed(Arc::new(submit)),
    ];

    // Config
    let config = RigAgentConfig::new(
        "intent-agent".to_string(),
        "Intent Analyzer".to_string(),
        "Analyzes user intent".to_string(),
        super::prompt::INTENT_AGENT_PROMPT.to_string(),
        RigModelConfig {
            provider_id: deps
                .provider
                .id
                .clone()
                .unwrap_or_else(|| "intent".to_string()),
            base_url: deps.provider.base_url.clone(),
            api_key: deps.provider.api_key.clone(),
            model: deps.model.clone(),
            temperature: 0.1,
            max_tokens: deps.max_tokens,
            context_window_tokens: 32_768,
            thinking_enabled: true,
            provider_params: None,
        },
    );

    // Shared context (minimal — intent agent has no session state)
    let shared = Arc::new(ToolContext::full_with_state(
        "intent-agent".to_string(),
        Some(format!("intent-{}", uuid::Uuid::new_v4())),
        vec![],
        HashMap::new(),
    ));

    // Build and run
    let engine =
        agent_runtime::rig_adapter::engine::RigAgentEngine::new(config, model, tools, shared);
    let _ = engine.execute_stream(message, &[], &mut |_| {}).await;

    // Extract the submitted analysis
    let result = result_slot.lock().unwrap().take();
    result
}
