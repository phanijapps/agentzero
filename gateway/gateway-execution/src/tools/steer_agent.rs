//! # Steer Agent Tool
//!
//! Sends mid-run instructions to a delegated subagent via its SteeringHandle.

use agent_primitives::{AgentError, Result, Tool, ToolContext};
use agent_runtime::{SteerResult, SteeringRegistry};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct SteerAgentTool {
    registry: Arc<SteeringRegistry>,
}

impl SteerAgentTool {
    pub fn new(registry: Arc<SteeringRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SteerAgentTool {
    fn name(&self) -> &'static str {
        "steer_agent"
    }

    fn description(&self) -> &'static str {
        "Send a mid-run instruction to a currently running delegated subagent. \
         Use the execution_id returned by delegate_to_agent. \
         The message is injected before the subagent's next LLM call. \
         Returns agent_not_running if the agent has already completed."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "execution_id": {
                    "type": "string",
                    "description": "The execution_id returned by delegate_to_agent"
                },
                "message": {
                    "type": "string",
                    "description": "Instruction to inject into the running subagent. Be concise — this appears as a system message."
                }
            },
            "required": ["execution_id", "message"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let execution_id = args
            .get("execution_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("execution_id is required".to_string()))?;

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("message is required".to_string()))?;

        const MAX_STEER_CHARS: usize = 1000;
        if message.len() > MAX_STEER_CHARS {
            return Err(AgentError::Tool(format!(
                "Steering message too large ({} chars). Maximum is {}. Be concise.",
                message.len(),
                MAX_STEER_CHARS
            )));
        }

        match self.registry.steer(execution_id, message) {
            SteerResult::Delivered => Ok(json!({
                "status": "delivered",
                "execution_id": execution_id,
                "message": "Steering instruction delivered. The subagent will process it before its next LLM call."
            })),
            SteerResult::AgentNotRunning => Ok(json!({
                "status": "agent_not_running",
                "execution_id": execution_id,
                "message": "Agent is not running (completed, failed, or unknown execution_id)."
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::steering::SteeringQueue;

    fn dummy_ctx() -> Arc<dyn agent_primitives::ToolContext> {
        Arc::new(agent_runtime::ToolContext::full_with_state(
            "test-agent".to_string(),
            None,
            vec![],
            std::collections::HashMap::new(),
        ))
    }

    #[tokio::test]
    async fn steer_delivers_to_running_agent() {
        let (mut queue, handle) = SteeringQueue::new();
        let registry = SteeringRegistry::new();
        registry.register("exec-abc", handle);
        let tool = SteerAgentTool::new(Arc::new(registry));

        let result = tool
            .execute(
                dummy_ctx(),
                json!({ "execution_id": "exec-abc", "message": "switch to plan B" }),
            )
            .await
            .unwrap();

        assert_eq!(result["status"], "delivered");

        let messages = queue.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "switch to plan B");
    }

    #[tokio::test]
    async fn steer_unknown_execution_id_returns_not_running() {
        let registry = SteeringRegistry::new();
        let tool = SteerAgentTool::new(Arc::new(registry));

        let result = tool
            .execute(
                dummy_ctx(),
                json!({ "execution_id": "exec-unknown", "message": "hello?" }),
            )
            .await
            .unwrap();

        assert_eq!(result["status"], "agent_not_running");
    }

    #[tokio::test]
    async fn oversized_message_returns_error() {
        let registry = SteeringRegistry::new();
        let tool = SteerAgentTool::new(Arc::new(registry));

        let big = "x".repeat(1001);
        let err = tool
            .execute(
                dummy_ctx(),
                json!({ "execution_id": "exec-abc", "message": big }),
            )
            .await
            .unwrap_err();

        assert!(format!("{err}").contains("too large"));
    }

    #[tokio::test]
    async fn missing_execution_id_returns_error() {
        let registry = SteeringRegistry::new();
        let tool = SteerAgentTool::new(Arc::new(registry));

        let err = tool
            .execute(dummy_ctx(), json!({ "message": "hello" }))
            .await
            .unwrap_err();

        assert!(format!("{err}").contains("execution_id"));
    }

    #[tokio::test]
    async fn missing_message_returns_error() {
        let registry = SteeringRegistry::new();
        let tool = SteerAgentTool::new(Arc::new(registry));

        let err = tool
            .execute(dummy_ctx(), json!({ "execution_id": "exec-abc" }))
            .await
            .unwrap_err();

        assert!(format!("{err}").contains("message"));
    }
}
