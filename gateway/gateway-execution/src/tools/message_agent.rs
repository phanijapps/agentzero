use crate::peer_messaging::{
    DurablePeerMessageService, PeerMessageContext, PeerMessageEnqueueError, MAX_PEER_MESSAGE_BYTES,
    PEER_MESSAGE_TARGET,
};
use agent_primitives::{AgentError, Result, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct MessageAgentTool {
    service: Arc<DurablePeerMessageService>,
}

impl MessageAgentTool {
    pub fn new(service: Arc<DurablePeerMessageService>) -> Self {
        Self { service }
    }
}

pub(super) fn peer_context(ctx: &dyn ToolContext) -> Result<PeerMessageContext> {
    fn required_state(ctx: &dyn ToolContext, key: &str) -> Result<String> {
        ctx.get_state(key)
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AgentError::Tool(format!("{key} is required in tool context")))
    }
    Ok(PeerMessageContext {
        node_id: PEER_MESSAGE_TARGET.to_owned(),
        agent_id: ctx.agent_name().to_owned(),
        session_id: required_state(ctx, "session_id")?,
        execution_id: required_state(ctx, "execution_id")?,
    })
}

pub(super) fn error_result(error: PeerMessageEnqueueError) -> Value {
    let (status, message) = match error {
        PeerMessageEnqueueError::InvalidRequest => {
            ("invalid_request", "The peer message request is invalid.")
        }
        PeerMessageEnqueueError::NotAuthorized => (
            "not_authorized",
            "The current execution is not authorized for this peer message.",
        ),
        PeerMessageEnqueueError::TargetNotFound => (
            "target_not_found",
            "The target is unavailable or outside the current session.",
        ),
        PeerMessageEnqueueError::TemporarilyUnavailable => (
            "temporarily_unavailable",
            "Peer messaging is temporarily unavailable.",
        ),
    };
    json!({"status": status, "message": message})
}

#[async_trait]
impl Tool for MessageAgentTool {
    fn name(&self) -> &'static str {
        "message_agent"
    }

    fn description(&self) -> &'static str {
        "Queue a durable asynchronous message for a running agent in this session. \
         Use list_session_agents to find its execution_id. This call does not wait."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "execution_id": {"type": "string", "description": "Current-session execution_id"},
                "message": {"type": "string", "minLength": 1, "maxLength": 1000, "description": "Peer message (at most 4,000 UTF-8 bytes)"}
            },
            "required": ["execution_id", "message"],
            "additionalProperties": false
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let execution_id = args
            .get("execution_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Tool("execution_id is required".to_owned()))?;
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| AgentError::Tool("message is required".to_owned()))?;
        if message.len() > MAX_PEER_MESSAGE_BYTES {
            return Ok(error_result(PeerMessageEnqueueError::InvalidRequest));
        }
        match self
            .service
            .enqueue_message(peer_context(ctx.as_ref())?, execution_id, message)
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
