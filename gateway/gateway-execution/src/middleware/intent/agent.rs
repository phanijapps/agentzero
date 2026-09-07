//! The intent agent — one Rig turn with discovery tools and a submit tool.
//!
//! The model reasons about the user's request, queries available resources
//! via tools, then calls `submit_intent` with the structured analysis.
//! No forced JSON, no structured-output path — the tool call IS the contract.

use super::contract::IntentAnalysis;
use agent_primitives::{Tool, ToolContext as ToolContextTrait};
use agent_runtime::llm::LlmClient;
use agent_runtime::rig_adapter::model::LlmCompletionModel;
use agent_runtime::rig_adapter::RigAgentConfig;
use agent_runtime::rig_adapter::RigModelConfig;
use agent_runtime::AgentEngine;
use gateway_services::providers::Provider;
use gateway_services::{AgentService, SharedVaultPaths, SkillService};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbot_stores::MemoryFactStore;
use zbot_stores_traits::ProcedureStore;

// ---------------------------------------------------------------------------
// Submit tool — the ONLY output mechanism
// ---------------------------------------------------------------------------

struct SubmitIntentTool {
    result: Arc<Mutex<Option<IntentAnalysis>>>,
}

impl SubmitIntentTool {
    fn new() -> (Self, Arc<Mutex<Option<IntentAnalysis>>>) {
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
        "Submit your intent analysis. This is the ONLY output — call exactly once."
    }

    fn parameters_schema(&self) -> Option<Value> {
        serde_json::to_value(schemars::schema_for!(IntentAnalysis)).ok()
    }

    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        args: Value,
    ) -> agent_primitives::Result<Value> {
        let analysis: IntentAnalysis = serde_json::from_value(args).map_err(|e| {
            agent_primitives::error::AgentError::Tool(format!("invalid intent analysis: {e}"))
        })?;
        *self.result.lock().unwrap() = Some(analysis);
        Ok(serde_json::json!({"status": "submitted"}))
    }
}

// ---------------------------------------------------------------------------
// Discovery tools — the agent queries resources on demand
// ---------------------------------------------------------------------------

struct ListSkillsTool {
    skill_service: Arc<SkillService>,
}

#[async_trait::async_trait]
impl Tool for ListSkillsTool {
    fn name(&self) -> &'static str {
        "list_skills"
    }
    fn description(&self) -> &'static str {
        "List all available skills with their descriptions."
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        _args: Value,
    ) -> agent_primitives::Result<Value> {
        let skills = self
            .skill_service
            .list()
            .await
            .map_err(|e| agent_primitives::error::AgentError::Tool(e.to_string()))?;
        Ok(serde_json::json!({
            "skills": skills.iter().map(|s| serde_json::json!({
                "name": s.name,
                "description": s.description,
            })).collect::<Vec<_>>()
        }))
    }
}

struct ListAgentsTool {
    agent_service: Arc<AgentService>,
}

#[async_trait::async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &'static str {
        "list_agents"
    }
    fn description(&self) -> &'static str {
        "List all available agents with their descriptions."
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        _args: Value,
    ) -> agent_primitives::Result<Value> {
        let agents = self
            .agent_service
            .list()
            .await
            .map_err(|e| agent_primitives::error::AgentError::Tool(e.to_string()))?;
        Ok(serde_json::json!({
            "agents": agents.iter().map(|a| serde_json::json!({
                "id": a.id,
                "name": a.name,
                "description": a.description,
            })).collect::<Vec<_>>()
        }))
    }
}

struct SearchProceduresTool {
    procedure_store: Option<Arc<dyn ProcedureStore>>,
}

#[async_trait::async_trait]
impl Tool for SearchProceduresTool {
    fn name(&self) -> &'static str {
        "search_procedures"
    }
    fn description(&self) -> &'static str {
        "Search learned procedures by name similarity."
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to search for (e.g. 'stock analysis workflow')"
                }
            },
            "required": ["query"]
        }))
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        args: Value,
    ) -> agent_primitives::Result<Value> {
        let Some(store) = &self.procedure_store else {
            return Ok(serde_json::json!({"procedures": []}));
        };
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or_default();
        let names = store
            .list_procedure_names("root", 20)
            .await
            .unwrap_or_default();
        // Filter by query substring for now; embedding search is a future enhancement
        let matched: Vec<_> = names
            .into_iter()
            .filter(|(name, _)| {
                name.to_lowercase().contains(&query.to_lowercase())
                    || query.to_lowercase().contains(&name.to_lowercase())
            })
            .map(|(name, ward_id)| serde_json::json!({"name": name, "ward_id": ward_id}))
            .collect();
        Ok(serde_json::json!({"procedures": matched}))
    }
}

struct ListWardsTool {
    paths: SharedVaultPaths,
}

#[async_trait::async_trait]
impl Tool for ListWardsTool {
    fn name(&self) -> &'static str {
        "list_wards"
    }
    fn description(&self) -> &'static str {
        "List all existing wards with their purpose."
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        _args: Value,
    ) -> agent_primitives::Result<Value> {
        let wards = super::router::list_wards(&self.paths);
        Ok(serde_json::json!({
            "wards": wards.iter().map(|(name, purpose)| serde_json::json!({
                "name": name,
                "purpose": purpose,
            })).collect::<Vec<_>>()
        }))
    }
}

struct SearchMemoryTool {
    fact_store: Arc<dyn MemoryFactStore>,
}

#[async_trait::async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &'static str {
        "search_memory"
    }
    fn description(&self) -> &'static str {
        "Search memory for relevant facts and patterns."
    }
    fn parameters_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to search for in memory"
                }
            },
            "required": ["query"]
        }))
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn ToolContextTrait>,
        args: Value,
    ) -> agent_primitives::Result<Value> {
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or_default();
        let result = self
            .fact_store
            .recall_facts("root", query, 10)
            .await
            .map_err(|e| agent_primitives::error::AgentError::Tool(e.to_string()))?;
        let items = result
            .get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(serde_json::json!({
            "memories": items.iter().take(5).map(|item| serde_json::json!({
                "content": item.get("content").and_then(|c| c.as_str()).unwrap_or(""),
                "category": item.get("category").and_then(|c| c.as_str()).unwrap_or(""),
            })).collect::<Vec<_>>()
        }))
    }
}

// ---------------------------------------------------------------------------
// The intent agent — one Rig turn
// ---------------------------------------------------------------------------

pub struct IntentAgentDeps {
    pub skill_service: Arc<SkillService>,
    pub agent_service: Arc<AgentService>,
    pub procedure_store: Option<Arc<dyn ProcedureStore>>,
    pub fact_store: Arc<dyn MemoryFactStore>,
    pub paths: SharedVaultPaths,
    pub provider: Provider,
    pub model: String,
    pub max_tokens: u64,
}

/// Run the intent agent: one Rig turn with discovery tools + submit_intent.
/// The model reasons, queries resources, then submits the analysis.
/// Returns None if the model didn't call submit_intent.
pub(crate) async fn run_intent_agent(
    deps: &IntentAgentDeps,
    message: &str,
    _on_event: impl FnMut(agent_runtime::StreamEvent) + Send,
) -> Option<IntentAnalysis> {
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

    // Build the tools
    let (submit_tool, result_slot) = SubmitIntentTool::new();
    let tools: Vec<std::sync::Arc<dyn agent_primitives::Tool>> = vec![
        Arc::new(ListSkillsTool {
            skill_service: deps.skill_service.clone(),
        }),
        Arc::new(ListAgentsTool {
            agent_service: deps.agent_service.clone(),
        }),
        Arc::new(SearchProceduresTool {
            procedure_store: deps.procedure_store.clone(),
        }),
        Arc::new(ListWardsTool {
            paths: deps.paths.clone(),
        }),
        Arc::new(SearchMemoryTool {
            fact_store: deps.fact_store.clone(),
        }),
        Arc::new(submit_tool),
    ];

    // Build the agent config
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

    // Minimal shared context — the intent agent doesn't need session state
    let shared_context = Arc::new(agent_runtime::ToolContext::full_with_state(
        "intent-agent".to_string(),
        Some(format!("intent-{}", uuid::Uuid::new_v4())),
        vec![],
        HashMap::new(),
    ));

    // Build and run the engine in a scope so it drops before we read the result
    let run_result = {
        let engine = agent_runtime::rig_adapter::factory::build_simple_engine(
            rig_config,
            completion_model,
            tools,
            shared_context,
        );
        engine.execute(message, &[]).await
    };

    if let Err(e) = &run_result {
        tracing::warn!(error = %e, "Intent agent execution failed");
    }

    // Extract the submitted analysis — result_slot outlives the engine
    let result = result_slot.lock().unwrap().take();
    result
}
