use super::list_zbots::{delegation_context, delegation_error};
use crate::a2a::A2aDelegationService;
use agent_primitives::{AgentError, Result, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct DelegateToZbotTool {
    service: Arc<dyn A2aDelegationService>,
}

impl DelegateToZbotTool {
    pub fn new(service: Arc<dyn A2aDelegationService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Tool for DelegateToZbotTool {
    fn name(&self) -> &'static str {
        "delegate_to_zbot"
    }

    fn description(&self) -> &'static str {
        "Queue bounded text work for an explicitly trusted zBot. Returns immediately with a stable task_id."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "peer_id": {"type": "string", "minLength": 1, "maxLength": 120},
                "content": {"type": "string", "minLength": 1, "maxLength": 1000, "description": "At most 4,000 UTF-8 bytes"}
            },
            "required": ["peer_id", "content"],
            "additionalProperties": false
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let peer_id = args
            .get("peer_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Tool("peer_id is required".to_owned()))?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Tool("content is required".to_owned()))?;
        match self
            .service
            .delegate(delegation_context(ctx.as_ref())?, peer_id, content)
            .await
        {
            Ok(receipt) => Ok(json!({"status": "queued", "task_id": receipt.task_id})),
            Err(error) => Ok(delegation_error(error)),
        }
    }
}
