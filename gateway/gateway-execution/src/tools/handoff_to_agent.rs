use agent_primitives::{AgentError, Result, Tool, ToolContext};
use agent_runtime::{SteerResult, SteeringRegistry};
use async_trait::async_trait;
use execution_state::{DelegationType, ExecutionStatus, StateService};
use serde_json::{json, Value};
use std::sync::Arc;
use zbot_stores_sqlite::DatabaseManager;

const MAX_HANDOFF_CHARS: usize = 1000;

pub struct HandoffToAgentTool {
    state_service: Arc<StateService<DatabaseManager>>,
    registry: Arc<SteeringRegistry>,
}

impl HandoffToAgentTool {
    pub fn new(
        state_service: Arc<StateService<DatabaseManager>>,
        registry: Arc<SteeringRegistry>,
    ) -> Self {
        Self {
            state_service,
            registry,
        }
    }
}

#[async_trait]
impl Tool for HandoffToAgentTool {
    fn name(&self) -> &'static str {
        "handoff_to_agent"
    }

    fn description(&self) -> &'static str {
        "Send a one-way handoff note to a running delegated agent in the current session. \
         Use list_session_agents to find the execution_id. \
         This does not wait for a reply; use wait_agent to retrieve completed results."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "execution_id": {
                    "type": "string",
                    "description": "The current-session execution_id returned by delegate_to_agent or list_session_agents"
                },
                "message": {
                    "type": "string",
                    "description": "Concise one-way handoff note. This is injected before the target agent's next LLM call."
                }
            },
            "required": ["execution_id", "message"]
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> Result<Value> {
        let session_id = context_session_id(ctx.as_ref())?;
        let execution_id = args
            .get("execution_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("execution_id is required".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AgentError::Tool("message is required".to_string()))?;

        if message.len() > MAX_HANDOFF_CHARS {
            return Err(AgentError::Tool(format!(
                "Handoff message too large ({} chars). Maximum is {}. Be concise.",
                message.len(),
                MAX_HANDOFF_CHARS
            )));
        }

        let Some(exec) = self
            .state_service
            .get_execution(execution_id)
            .map_err(|e| AgentError::Tool(format!("failed to load execution: {e}")))?
        else {
            return Ok(json!({
                "status": "target_not_found",
                "execution_id": execution_id,
                "message": "Execution is unknown or outside the current session."
            }));
        };

        if exec.session_id != session_id || exec.delegation_type == DelegationType::Root {
            return Ok(json!({
                "status": "target_not_found",
                "execution_id": execution_id,
                "message": "Execution is unknown or outside the current session."
            }));
        }

        if exec.status != ExecutionStatus::Running {
            return Ok(json!({
                "status": "agent_not_running",
                "execution_id": execution_id,
                "message": "Agent is not running. Use wait_agent to read completed results."
            }));
        }

        let note = format!("[Handoff note from orchestrator] {message}");
        match self.registry.steer(execution_id, note) {
            SteerResult::Delivered => Ok(json!({
                "status": "delivered",
                "execution_id": execution_id,
                "message": "Handoff note queued. The agent will process it before its next LLM call."
            })),
            SteerResult::AgentNotRunning => Ok(json!({
                "status": "agent_not_running",
                "execution_id": execution_id,
                "message": "Agent is not running. Use wait_agent to read completed results."
            })),
        }
    }
}

fn context_session_id(ctx: &dyn ToolContext) -> Result<String> {
    ctx.get_state("session_id")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .filter(|session_id| !session_id.is_empty())
        .ok_or_else(|| AgentError::Tool("session_id is required in tool context".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::steering::SteeringQueue;
    use execution_state::StateService;
    use gateway_services::VaultPaths;
    use serde_json::json;
    use std::collections::HashMap;
    use tempfile::TempDir;

    struct Harness {
        _tmp: TempDir,
        state_service: Arc<StateService<DatabaseManager>>,
        registry: Arc<SteeringRegistry>,
        session_id: String,
        root_execution_id: String,
    }

    fn setup() -> Harness {
        let tmp = TempDir::new().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("ensure vault dirs");
        let db = Arc::new(DatabaseManager::new(paths).expect("db init"));
        let state_service = Arc::new(StateService::new(db));
        let (session, root) = state_service.create_session("root").unwrap();
        Harness {
            _tmp: tmp,
            state_service,
            registry: Arc::new(SteeringRegistry::new()),
            session_id: session.id,
            root_execution_id: root.id,
        }
    }

    fn ctx_with_session(session_id: &str) -> Arc<dyn ToolContext> {
        let mut state = HashMap::new();
        state.insert(
            "session_id".to_string(),
            Value::String(session_id.to_string()),
        );
        Arc::new(agent_runtime::ToolContext::full_with_state(
            "test-root".to_string(),
            Some("legacy-conv-id".to_string()),
            vec![],
            state,
        ))
    }

    fn ctx_without_session() -> Arc<dyn ToolContext> {
        Arc::new(agent_runtime::ToolContext::full_with_state(
            "test-root".to_string(),
            Some("legacy-conv-id".to_string()),
            vec![],
            HashMap::new(),
        ))
    }

    fn running_child(h: &Harness, execution_id: &str) {
        h.state_service
            .create_delegated_execution_with_id(
                execution_id,
                &h.session_id,
                "research-agent",
                &h.root_execution_id,
                execution_state::DelegationType::Parallel,
                "research",
            )
            .unwrap();
        h.state_service.start_execution(execution_id).unwrap();
    }

    #[tokio::test]
    async fn handoff_to_agent_delivers_to_current_session_running_agent() {
        let h = setup();
        running_child(&h, "exec-running");
        let (mut queue, handle) = SteeringQueue::new();
        h.registry.register("exec-running", handle);
        let tool = HandoffToAgentTool::new(h.state_service, h.registry);

        let result = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "execution_id": "exec-running", "message": "use source A" }),
            )
            .await
            .unwrap();

        assert_eq!(result["status"], "delivered");
        assert_eq!(result["execution_id"], "exec-running");
        let messages = queue.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].content,
            "[Handoff note from orchestrator] use source A"
        );
    }

    #[tokio::test]
    async fn handoff_to_agent_returns_not_running_for_terminal_statuses() {
        for status in [
            ExecutionStatus::Completed,
            ExecutionStatus::Crashed,
            ExecutionStatus::Cancelled,
        ] {
            let h = setup();
            running_child(&h, "exec-terminal");
            match status {
                ExecutionStatus::Completed => {
                    h.state_service.complete_execution("exec-terminal").unwrap()
                }
                ExecutionStatus::Crashed => h
                    .state_service
                    .crash_execution("exec-terminal", "boom")
                    .unwrap(),
                ExecutionStatus::Cancelled => {
                    h.state_service.cancel_execution("exec-terminal").unwrap()
                }
                _ => unreachable!(),
            }

            let (mut queue, handle) = SteeringQueue::new();
            h.registry.register("exec-terminal", handle);
            let tool = HandoffToAgentTool::new(h.state_service, h.registry);
            let result = tool
                .execute(
                    ctx_with_session(&h.session_id),
                    json!({ "execution_id": "exec-terminal", "message": "too late" }),
                )
                .await
                .unwrap();

            assert_eq!(result["status"], "agent_not_running");
            assert!(queue.drain().is_empty());
        }
    }

    #[tokio::test]
    async fn handoff_to_agent_returns_not_running_when_registry_missing() {
        let h = setup();
        running_child(&h, "exec-no-handle");
        let tool = HandoffToAgentTool::new(h.state_service, h.registry);

        let result = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "execution_id": "exec-no-handle", "message": "hello" }),
            )
            .await
            .unwrap();

        assert_eq!(result["status"], "agent_not_running");
    }

    #[tokio::test]
    async fn handoff_to_agent_rejects_cross_session_execution_id() {
        let h = setup();
        let (other_session, other_root) = h.state_service.create_session("root").unwrap();
        h.state_service
            .create_delegated_execution_with_id(
                "exec-other",
                &other_session.id,
                "builder-agent",
                &other_root.id,
                execution_state::DelegationType::Parallel,
                "build",
            )
            .unwrap();
        h.state_service.start_execution("exec-other").unwrap();
        let (mut queue, handle) = SteeringQueue::new();
        h.registry.register("exec-other", handle);
        let tool = HandoffToAgentTool::new(h.state_service, h.registry);

        let result = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "execution_id": "exec-other", "message": "wrong session" }),
            )
            .await
            .unwrap();

        assert_eq!(result["status"], "target_not_found");
        assert!(queue.drain().is_empty());
    }

    #[tokio::test]
    async fn handoff_to_agent_rejects_root_execution_id() {
        let h = setup();
        h.registry
            .register(&h.root_execution_id, SteeringQueue::new().1);
        let tool = HandoffToAgentTool::new(h.state_service, h.registry);

        let result = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "execution_id": h.root_execution_id, "message": "not a child" }),
            )
            .await
            .unwrap();

        assert_eq!(result["status"], "target_not_found");
    }

    #[tokio::test]
    async fn handoff_to_agent_returns_target_not_found_for_unknown_execution_id() {
        let h = setup();
        let tool = HandoffToAgentTool::new(h.state_service, h.registry);

        let result = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "execution_id": "exec-missing", "message": "hello" }),
            )
            .await
            .unwrap();

        assert_eq!(result["status"], "target_not_found");
    }

    #[tokio::test]
    async fn handoff_to_agent_validates_required_args_and_size() {
        let h = setup();
        let tool = HandoffToAgentTool::new(h.state_service, h.registry);

        let missing_execution = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "message": "hello" }),
            )
            .await
            .unwrap_err();
        assert!(format!("{missing_execution}").contains("execution_id"));

        let missing_message = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "execution_id": "exec-1" }),
            )
            .await
            .unwrap_err();
        assert!(format!("{missing_message}").contains("message"));

        let oversized = tool
            .execute(
                ctx_with_session(&h.session_id),
                json!({ "execution_id": "exec-1", "message": "x".repeat(MAX_HANDOFF_CHARS + 1) }),
            )
            .await
            .unwrap_err();
        assert!(format!("{oversized}").contains("too large"));
    }

    #[tokio::test]
    async fn handoff_to_agent_requires_context_session_id() {
        let h = setup();
        let tool = HandoffToAgentTool::new(h.state_service, h.registry);

        let err = tool
            .execute(
                ctx_without_session(),
                json!({ "execution_id": "exec-1", "message": "hello" }),
            )
            .await
            .unwrap_err();

        assert!(format!("{err}").contains("session_id"));
    }
}
