//! The intent agent — one Rig turn with a semantic search tool and a submit
//! tool. The agent reasons about the request, searches the index for
//! relevant resources, then calls `submit_intent` with the analysis.

use super::contract::IntentAnalysis;
use agent_primitives::error::AgentError;
use agent_primitives::{Tool, ToolContext as ToolContextTrait};
use agent_runtime::llm::LlmClient;
use agent_runtime::rig_adapter::model::LlmCompletionModel;
use agent_runtime::rig_adapter::{RigAgentConfig, RigModelConfig};
use agent_runtime::AgentEngine;
use gateway_services::providers::Provider;
use gateway_services::SharedVaultPaths;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbot_stores::MemoryFactStore;
use zbot_stores_traits::ProcedureStore;

// ---------------------------------------------------------------------------
// Search tool — semantic search over the indexed resources
// ---------------------------------------------------------------------------

struct SearchIndexTool {
    fact_store: Arc<dyn MemoryFactStore>,
}

#[async_trait::async_trait]
impl Tool for SearchIndexTool {
    fn name(&self) -> &'static str {
        "search_index"
    }
    fn description(&self) -> &'static str {
        "Search the indexed resources (skills, agents, wards, MCPs, procedures, \
         memories) for entries relevant to a query. Use this to discover what \
         is available before deciding how to route the request."
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to search for (e.g. 'financial analysis skills', 'research agents', 'stock valuation procedures')"
                },
                "category": {
                    "type": "string",
                    "enum": ["skill", "agent", "ward", "mcp", "procedure", "any"],
                    "description": "Filter to a resource type, or 'any' for all"
                }
            },
            "required": ["query"]
        }))
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        args: Value,
    ) -> agent_primitives::error::Result<Value> {
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or_default();
        let category_filter = args
            .get("category")
            .and_then(|c| c.as_str())
            .unwrap_or("any");

        let result = self
            .fact_store
            .recall_facts("root", query, 20)
            .await
            .map_err(|e| AgentError::Tool(e.to_string()))?;

        let items = result
            .get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let filtered: Vec<_> = items
            .into_iter()
            .filter(|item| {
                if category_filter == "any" {
                    true
                } else {
                    item.get("category")
                        .and_then(|c| c.as_str())
                        .is_some_and(|c| c == category_filter)
                }
            })
            .take(10)
            .map(|item| {
                let key = item.get("key").and_then(|k| k.as_str()).unwrap_or("");
                let content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
                let category = item.get("category").and_then(|c| c.as_str()).unwrap_or("");
                // Strip the key prefix for clean display
                let name = key.split(':').nth(1).unwrap_or(key);
                serde_json::json!({
                    "name": name,
                    "description": content,
                    "category": category,
                })
            })
            .collect();

        Ok(serde_json::json!({ "results": filtered }))
    }
}

// ---------------------------------------------------------------------------
// Submit tool — the ONLY output mechanism
// ---------------------------------------------------------------------------

struct SubmitIntentTool {
    result: Arc<Mutex<Option<IntentAnalysis>>>,
}

#[async_trait::async_trait]
impl Tool for SubmitIntentTool {
    fn name(&self) -> &'static str {
        "submit_intent"
    }
    fn description(&self) -> &'static str {
        "Submit your intent analysis. This is the ONLY way to complete. \
         Call this exactly once after reasoning about the request."
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
// The intent agent — one Rig turn
// ---------------------------------------------------------------------------

pub struct IntentAgentDeps {
    pub fact_store: Arc<dyn MemoryFactStore>,
    pub procedure_store: Option<Arc<dyn ProcedureStore>>,
    pub paths: SharedVaultPaths,
    pub provider: Provider,
    pub model: String,
    pub max_tokens: u64,
}

/// Run the intent agent: one Rig turn with search_index + submit_intent.
/// The model reasons, searches the index, then submits the analysis.
pub async fn run_intent_agent(deps: &IntentAgentDeps, message: &str) -> Option<IntentAnalysis> {
    // Build the LLM client from the configured intent model
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
    let completion_model = LlmCompletionModel::new(client, deps.model.clone());

    // Two tools: search the index, submit the analysis
    let (submit_tool, result_slot) = SubmitIntentTool {
        result: Arc::new(Mutex::new(None)),
    }
    .split();
    let tools: Vec<std::sync::Arc<dyn agent_primitives::Tool>> = vec![
        Arc::new(SearchIndexTool {
            fact_store: deps.fact_store.clone(),
        }),
        Arc::new(submit_tool),
    ];

    // Agent config
    let rig_config = RigAgentConfig::new(
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

    let shared_context = Arc::new(agent_runtime::ToolContext::full_with_state(
        "intent-agent".to_string(),
        Some(format!("intent-{}", uuid::Uuid::new_v4())),
        vec![],
        HashMap::new(),
    ));

    // Run the agent — the engine drives the tool loop
    let run_result = {
        let engine = agent_runtime::rig_adapter::factory::build_simple_engine(
            rig_config,
            completion_model,
            tools,
            shared_context,
        );
        engine.execute_stream(message, &[], &mut |_| {}).await
    };

    if let Err(e) = &run_result {
        tracing::warn!(error = %e, "Intent agent execution failed");
    }

    let result = result_slot.lock().unwrap().take();
    result
}

impl SubmitIntentTool {
    fn split(self) -> (Self, Arc<Mutex<Option<IntentAnalysis>>>) {
        let slot = self.result.clone();
        (self, slot)
    }
}
