use super::contract::IntentAnalysis;
use gateway_services::providers::Provider;
use gateway_services::SharedVaultPaths;
use std::sync::{Arc, Mutex};
use zbot_stores::MemoryFactStore;
use zbot_stores_traits::ProcedureStore;

// ---------------------------------------------------------------------------
// Submit tool — the model calls this with its IntentAnalysis
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
impl agent_primitives::Tool for SubmitIntentTool {
    fn name(&self) -> &'static str {
        "submit_intent"
    }
    fn description(&self) -> &'static str {
        "Submit your intent analysis. This is the ONLY way to complete."
    }
    fn parameters_schema(&self) -> Option<serde_json::Value> {
        serde_json::to_value(schemars::schema_for!(IntentAnalysis)).ok()
    }
    async fn execute(
        &self,
        _ctx: Arc<dyn agent_primitives::ToolContext>,
        args: serde_json::Value,
    ) -> agent_primitives::error::Result<serde_json::Value> {
        let analysis: IntentAnalysis = serde_json::from_value(args).map_err(|e| {
            agent_primitives::error::AgentError::Tool(format!("invalid intent analysis: {e}"))
        })?;
        *self.result.lock().unwrap() = Some(analysis);
        Ok(serde_json::json!({"status": "submitted"}))
    }
}

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

    // Search the fact store for relevant resources
    let search_result = deps
        .fact_store
        .recall_facts("root", message, 20)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Intent search failed");
            serde_json::json!({"results": []})
        });

    let items = search_result
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let resource_context: Vec<String> = items
        .iter()
        .take(8)
        .filter_map(|item| {
            let key = item.get("key").and_then(|k| k.as_str())?;
            let content = item.get("content").and_then(|c| c.as_str())?;
            let category = item.get("category").and_then(|c| c.as_str()).unwrap_or("");
            let name = key.split(':').nth(1).unwrap_or(key);
            Some(format!("- [{}] {}: {}", category, name, content))
        })
        .collect();

    let user_prompt = format!(
        "Available resources:\n{}\n\nUser request: {}",
        if resource_context.is_empty() {
            "(none found)".to_string()
        } else {
            resource_context.join("\n")
        },
        message
    );

    // One plain chat call — the model returns JSON (proven by test).
    // response_format: json_schema is broken on Ollama, so we ask via prompt.
    let msgs = vec![
        agent_runtime::ChatMessage::system(super::prompt::INTENT_AGENT_PROMPT.to_string()),
        agent_runtime::ChatMessage::user(user_prompt),
    ];

    match client.chat(msgs, None).await {
        Ok(response) => {
            let content = response.content.trim();
            // Model returns JSON — parse directly
            match serde_json::from_str::<IntentAnalysis>(content) {
                Ok(analysis) => {
                    tracing::info!(
                        primary_intent = %analysis.primary_intent,
                        approach = %analysis.execution_strategy.approach,
                        "Intent agent complete"
                    );
                    Some(analysis)
                }
                Err(e) => {
                    // Try to find the JSON object in the response
                    // (model might wrap it in markdown fences)
                    let start = content.find('{')?;
                    let end = content.rfind('}')?;
                    match serde_json::from_str::<IntentAnalysis>(&content[start..=end]) {
                        Ok(analysis) => {
                            tracing::info!(
                                primary_intent = %analysis.primary_intent,
                                approach = %analysis.execution_strategy.approach,
                                "Intent agent complete (extracted from wrapped JSON)"
                            );
                            Some(analysis)
                        }
                        Err(e2) => {
                            tracing::warn!(error = %e2, "Intent JSON parse failed: {e}");
                            None
                        }
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "Intent agent LLM call failed");
            None
        }
    }
}
