// ============================================================================
// GOAL TOOL
// Create, update, list, and get agent goals. Active goals steer recall.
// ============================================================================

// Public API types — consumed by downstream (gateway) that wires a concrete
// GoalAccess into the tool. No in-crate caller yet.
#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use agent_primitives::{AgentError, Result, Tool, ToolContext};

/// Lightweight snapshot of a goal — enough for tool return payloads.
#[derive(Debug, Clone)]
pub struct GoalSummary {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub slots: Option<String>,        // JSON string
    pub filled_slots: Option<String>, // JSON string
}

/// Abstraction over the gateway's GoalRepository. Gateway wires a concrete impl.
#[async_trait]
pub trait GoalAccess: Send + Sync + 'static {
    async fn create(
        &self,
        agent_id: &str,
        title: &str,
        description: Option<&str>,
        slots_json: Option<&str>,
    ) -> std::result::Result<GoalSummary, String>;

    async fn update_state(&self, goal_id: &str, new_state: &str)
    -> std::result::Result<(), String>;

    async fn update_filled_slots(
        &self,
        goal_id: &str,
        filled_slots_json: &str,
    ) -> std::result::Result<(), String>;

    async fn list_active(&self, agent_id: &str) -> std::result::Result<Vec<GoalSummary>, String>;

    async fn get(&self, goal_id: &str) -> std::result::Result<Option<GoalSummary>, String>;
}

/// Tool that creates, updates, lists, and retrieves agent goals.
pub struct GoalTool {
    access: Arc<dyn GoalAccess>,
}

impl GoalTool {
    pub fn new(access: Arc<dyn GoalAccess>) -> Self {
        Self { access }
    }
}

#[async_trait]
impl Tool for GoalTool {
    fn name(&self) -> &str {
        "goal"
    }

    fn description(&self) -> &str {
        "Manage agent goals. Active goals steer recall — the memory layer \
         boosts items aligned with open goal slots."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "update_state", "update_slots", "list_active", "get"],
                    "description": "The action to perform."
                },
                "title": {
                    "type": "string",
                    "description": "Goal title (for create)."
                },
                "description": {
                    "type": "string",
                    "description": "Optional longer description (for create)."
                },
                "slots": {
                    "type": "string",
                    "description": "JSON array of slot definitions (for create), e.g. '[{\"name\":\"tickers\",\"type\":\"list\"}]'."
                },
                "id": {
                    "type": "string",
                    "description": "Goal id (for update_state, update_slots, get)."
                },
                "state": {
                    "type": "string",
                    "enum": ["active", "blocked", "satisfied", "abandoned"],
                    "description": "New state (for update_state)."
                },
                "filled_slots": {
                    "type": "string",
                    "description": "JSON object of slot name → value (for update_slots), e.g. '{\"tickers\":[\"AAPL\"]}'."
                }
            },
            "required": ["action"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("Missing 'action'".to_string()))?;

        match action {
            "create" => execute_create(&self.access, ctx, &args).await,
            "update_state" => execute_update_state(&self.access, &args).await,
            "update_slots" => execute_update_slots(&self.access, &args).await,
            "list_active" => execute_list_active(&self.access, ctx).await,
            "get" => execute_get(&self.access, &args).await,
            other => Err(AgentError::Tool(format!("Unknown action: {other}"))),
        }
    }
}

async fn execute_create(
    access: &Arc<dyn GoalAccess>,
    ctx: Arc<dyn ToolContext>,
    args: &Value,
) -> Result<Value> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::Tool("Missing 'title'".to_string()))?;
    let description = args.get("description").and_then(|v| v.as_str());
    let slots = args.get("slots").and_then(|v| v.as_str());
    let agent_id = ctx.agent_name().to_string();
    let summary = access
        .create(&agent_id, title, description, slots)
        .await
        .map_err(AgentError::Tool)?;
    Ok(summary_to_value(&summary))
}

async fn execute_update_state(access: &Arc<dyn GoalAccess>, args: &Value) -> Result<Value> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::Tool("Missing 'id'".to_string()))?;
    let state = args
        .get("state")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::Tool("Missing 'state'".to_string()))?;
    access
        .update_state(id, state)
        .await
        .map_err(AgentError::Tool)?;
    Ok(json!({"id": id, "state": state, "status": "updated"}))
}

async fn execute_update_slots(access: &Arc<dyn GoalAccess>, args: &Value) -> Result<Value> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::Tool("Missing 'id'".to_string()))?;
    let filled = args
        .get("filled_slots")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::Tool("Missing 'filled_slots'".to_string()))?;
    access
        .update_filled_slots(id, filled)
        .await
        .map_err(AgentError::Tool)?;
    Ok(json!({"id": id, "filled_slots": filled, "status": "updated"}))
}

async fn execute_list_active(
    access: &Arc<dyn GoalAccess>,
    ctx: Arc<dyn ToolContext>,
) -> Result<Value> {
    let agent_id = ctx.agent_name().to_string();
    let goals = access
        .list_active(&agent_id)
        .await
        .map_err(AgentError::Tool)?;
    Ok(json!({
        "goals": goals.iter().map(summary_to_value).collect::<Vec<_>>()
    }))
}

async fn execute_get(access: &Arc<dyn GoalAccess>, args: &Value) -> Result<Value> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AgentError::Tool("Missing 'id'".to_string()))?;
    match access.get(id).await.map_err(AgentError::Tool)? {
        Some(g) => Ok(summary_to_value(&g)),
        None => Ok(json!({"id": id, "found": false})),
    }
}

fn summary_to_value(g: &GoalSummary) -> Value {
    json!({
        "id": g.id,
        "title": g.title,
        "description": g.description,
        "state": g.state,
        "slots": g.slots,
        "filled_slots": g.filled_slots,
    })
}
