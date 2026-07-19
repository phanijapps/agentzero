use crate::agent_pool::AgentResultBus;
use agent_primitives::{AgentError, Result, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct KillAgentTool {
    bus: Arc<AgentResultBus>,
}

impl KillAgentTool {
    pub fn new(bus: Arc<AgentResultBus>) -> Self {
        Self { bus }
    }
}

#[async_trait]
impl Tool for KillAgentTool {
    fn name(&self) -> &'static str {
        "kill_agent"
    }

    fn description(&self) -> &'static str {
        "Stop a running delegated subagent immediately. \
         Pass the execution_id returned by delegate_to_agent. \
         Unblocks any pending wait_agent on this execution with a crashed error. \
         Returns status: stopped if the agent was running, not_running otherwise."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "execution_id": {
                    "type": "string",
                    "description": "The execution_id returned by delegate_to_agent"
                }
            },
            "required": ["execution_id"]
        }))
    }

    async fn execute(&self, _ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let execution_id = args
            .get("execution_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("execution_id is required".to_string()))?;

        if self.bus.kill(execution_id) {
            tracing::info!(execution_id = %execution_id, "kill_agent: stopped running subagent");
            Ok(json!({ "status": "stopped", "execution_id": execution_id }))
        } else {
            Ok(json!({ "status": "not_running", "execution_id": execution_id }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::ExecutionHandle;

    fn dummy_ctx() -> Arc<dyn agent_primitives::ToolContext> {
        Arc::new(agent_runtime::ToolContext::full_with_state(
            "test-agent".to_string(),
            None,
            vec![],
            std::collections::HashMap::new(),
        ))
    }

    #[tokio::test]
    async fn kill_agent_stops_registered_execution() {
        let bus = Arc::new(AgentResultBus::new());
        let handle = ExecutionHandle::new(10);
        bus.register_handle("exec-1", handle.clone());

        let tool = KillAgentTool::new(bus.clone());
        let result = tool
            .execute(dummy_ctx(), json!({ "execution_id": "exec-1" }))
            .await
            .unwrap();

        assert_eq!(result["status"], "stopped");
        assert_eq!(result["execution_id"], "exec-1");
        assert!(
            handle.is_stop_requested(),
            "kill must trigger handle.stop()"
        );
    }

    #[tokio::test]
    async fn kill_agent_returns_not_running_for_unknown_execution() {
        let bus = Arc::new(AgentResultBus::new());
        let tool = KillAgentTool::new(bus);

        let result = tool
            .execute(dummy_ctx(), json!({ "execution_id": "exec-ghost" }))
            .await
            .unwrap();

        assert_eq!(result["status"], "not_running");
    }

    #[tokio::test]
    async fn kill_agent_requires_execution_id_arg() {
        let bus = Arc::new(AgentResultBus::new());
        let tool = KillAgentTool::new(bus);

        let err = tool.execute(dummy_ctx(), json!({})).await.unwrap_err();
        assert!(format!("{err}").contains("execution_id"));
    }
}
