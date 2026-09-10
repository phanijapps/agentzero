use super::message_agent::{error_result, peer_context};
use crate::peer_messaging::DurablePeerMessageService;
use agent_primitives::{AgentError, Result, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct ReplyToAgentTool {
    service: Arc<DurablePeerMessageService>,
}

impl ReplyToAgentTool {
    pub fn new(service: Arc<DurablePeerMessageService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Tool for ReplyToAgentTool {
    fn name(&self) -> &'static str {
        "reply_to_agent"
    }

    fn description(&self) -> &'static str {
        "Queue a durable asynchronous reply using the reply token included in a peer message. \
         The token works only for the exact recipient and cannot choose another target."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "reply_to": {"type": "string", "description": "work-* reply token from the peer message"},
                "message": {"type": "string", "minLength": 1, "maxLength": 1000, "description": "Reply (at most 4,000 UTF-8 bytes)"}
            },
            "required": ["reply_to", "message"],
            "additionalProperties": false
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let reply_to = args
            .get("reply_to")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Tool("reply_to is required".to_owned()))?;
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Tool("message is required".to_owned()))?;
        match self
            .service
            .enqueue_reply(peer_context(ctx.as_ref())?, reply_to, message)
            .await
        {
            Ok(receipt) => Ok(json!({
                "status": "queued",
                "message_id": receipt.message_id,
                "execution_id": receipt.target_execution_id
            })),
            Err(error) => Ok(error_result(error)),
        }
    }
}
