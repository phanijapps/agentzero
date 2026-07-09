use crate::agent_pool::{AgentResultBus, AgentWaitError};
use agent_primitives::{AgentError, Result, Tool, ToolContext};
use async_trait::async_trait;
use execution_state::{ExecutionStatus, StateService};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::time::Duration;
use zbot_conversation::MessageStore;
use zbot_stores_sqlite::DatabaseManager;

pub struct WaitAgentTool {
    bus: Arc<AgentResultBus>,
    state_service: Arc<StateService<DatabaseManager>>,
    messages: Arc<dyn MessageStore>,
}

impl WaitAgentTool {
    pub fn new(
        bus: Arc<AgentResultBus>,
        state_service: Arc<StateService<DatabaseManager>>,
        messages: Arc<dyn MessageStore>,
    ) -> Self {
        Self {
            bus,
            state_service,
            messages,
        }
    }
}

#[async_trait]
impl Tool for WaitAgentTool {
    fn name(&self) -> &'static str {
        "wait_agent"
    }

    fn description(&self) -> &'static str {
        "Block until a delegated subagent completes and return its result. \
         Use this for fire-and-forget delegations; do not call it after \
         delegate_to_agent with wait_for_result=true, because that path auto-resumes \
         with the result. Pass the execution_id returned by delegate_to_agent. \
         Returns the agent's respond() text. Times out after timeout_secs (default 300). \
         Use this to coordinate fire-and-forget steps: delegate, wait, use result, delegate next."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "execution_id": {
                    "type": "string",
                    "description": "The execution_id returned by delegate_to_agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Seconds to wait before returning a timeout error (default: 300)"
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

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        // Fast path: agent already completed before wait_agent was called.
        if let Ok(Some(exec)) = self.state_service.get_execution(execution_id) {
            match exec.status {
                ExecutionStatus::Completed => {
                    let response = exec
                        .child_session_id
                        .as_deref()
                        .and_then(|sid| self.messages.replay(sid, None, 10).ok())
                        .and_then(|msgs| {
                            msgs.into_iter()
                                .rev()
                                .find(|m| m.role == "assistant")
                                .map(|m| m.content)
                        })
                        .unwrap_or_default();
                    return Ok(json!({
                        "execution_id": execution_id,
                        "result": response
                    }));
                }
                ExecutionStatus::Crashed | ExecutionStatus::Cancelled => {
                    return Ok(json!({
                        "error": "crashed",
                        "execution_id": execution_id,
                        "details": exec.error.unwrap_or_default()
                    }));
                }
                _ => {}
            }
        }

        // Register waiter and block until the execution resolves or timeout.
        let rx = self.bus.register_waiter(execution_id);

        match tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(Ok(result))) => Ok(json!({
                "execution_id": result.execution_id,
                "agent_id": result.agent_id,
                "result": result.response
            })),
            Ok(Ok(Err(AgentWaitError::Crashed { error }))) => Ok(json!({
                "error": "crashed",
                "execution_id": execution_id,
                "details": error
            })),
            Ok(Ok(Err(AgentWaitError::NotFound(id)))) => Ok(json!({
                "error": "not_found",
                "execution_id": id
            })),
            Ok(Ok(Err(AgentWaitError::Timeout))) => Ok(json!({
                "error": "timeout",
                "execution_id": execution_id
            })),
            Ok(Err(_)) => Ok(json!({
                "error": "crashed",
                "execution_id": execution_id,
                "details": "agent result channel closed unexpectedly"
            })),
            Err(_elapsed) => Ok(json!({
                "error": "timeout",
                "execution_id": execution_id
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::ExecutionHandle;
    use gateway_services::VaultPaths;
    use serde_json::json;
    use tempfile::TempDir;
    use zbot_stores_sqlite::DatabaseManager;

    struct Harness {
        _tmp: TempDir,
        bus: Arc<AgentResultBus>,
        state_service: Arc<StateService<DatabaseManager>>,
        messages: Arc<dyn MessageStore>,
    }

    fn setup() -> Harness {
        let tmp = TempDir::new().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("ensure vault dirs");
        let conversations_db = paths.conversations_db();
        let db = Arc::new(DatabaseManager::new(paths).expect("db init"));
        let state_service = Arc::new(StateService::new(db.clone()));
        let pool = zbot_conversation::open_conversation_pool(&conversations_db)
            .expect("conversation pool");
        let messages = Arc::new(zbot_conversation::SqliteMessageStore::new(pool));
        let bus = Arc::new(AgentResultBus::new());
        Harness {
            _tmp: tmp,
            bus,
            state_service,
            messages,
        }
    }

    fn dummy_ctx() -> Arc<dyn agent_primitives::ToolContext> {
        Arc::new(agent_runtime::ToolContext::full_with_state(
            "test-root".to_string(),
            None,
            vec![],
            std::collections::HashMap::new(),
        ))
    }

    #[tokio::test]
    async fn wait_agent_requires_execution_id_arg() {
        let h = setup();
        let tool = WaitAgentTool::new(h.bus, h.state_service, h.messages);
        let err = tool.execute(dummy_ctx(), json!({})).await.unwrap_err();
        assert!(format!("{err}").contains("execution_id"));
    }

    /// Slow path: agent finishes AFTER wait_agent registered. The tool
    /// blocks on the bus, then resolve unblocks it. Bypasses the
    /// state-service fast-path by using an execution_id the service
    /// doesn't know about (its lookup returns None).
    #[tokio::test]
    async fn wait_agent_blocks_then_unblocks_on_resolve() {
        let h = setup();
        let bus_for_resolver = h.bus.clone();
        let tool = WaitAgentTool::new(h.bus, h.state_service, h.messages);

        // Resolve after a short delay (long enough that the tool has
        // definitely entered timeout.await but well under the 300s
        // default).
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            bus_for_resolver.resolve("exec-not-in-state", "researcher", "found stuff");
        });

        let result = tool
            .execute(
                dummy_ctx(),
                json!({ "execution_id": "exec-not-in-state", "timeout_secs": 5 }),
            )
            .await
            .unwrap();

        assert_eq!(result["execution_id"], "exec-not-in-state");
        assert_eq!(result["agent_id"], "researcher");
        assert_eq!(result["result"], "found stuff");
        assert!(result.get("error").is_none());
    }

    /// Reject path: bus.reject delivers a Crashed error which the tool
    /// surfaces as `{"error": "crashed", ...}`.
    #[tokio::test]
    async fn wait_agent_surfaces_crash_from_reject() {
        let h = setup();
        let bus_for_rejecter = h.bus.clone();
        let tool = WaitAgentTool::new(h.bus, h.state_service, h.messages);

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            bus_for_rejecter.reject(
                "exec-crash",
                AgentWaitError::Crashed {
                    error: "shell command died".to_string(),
                },
            );
        });

        let result = tool
            .execute(
                dummy_ctx(),
                json!({ "execution_id": "exec-crash", "timeout_secs": 5 }),
            )
            .await
            .unwrap();

        assert_eq!(result["error"], "crashed");
        assert_eq!(result["execution_id"], "exec-crash");
        assert_eq!(result["details"], "shell command died");
    }

    /// Timeout path: no resolver fires, tokio::time::timeout returns
    /// Elapsed, tool surfaces it as `{"error": "timeout", ...}`.
    #[tokio::test]
    async fn wait_agent_returns_timeout_when_no_resolver() {
        let h = setup();
        let tool = WaitAgentTool::new(h.bus, h.state_service, h.messages);

        let result = tool
            .execute(
                dummy_ctx(),
                json!({ "execution_id": "exec-stalled", "timeout_secs": 1 }),
            )
            .await
            .unwrap();

        assert_eq!(result["error"], "timeout");
        assert_eq!(result["execution_id"], "exec-stalled");
    }

    /// Kill path: bus.kill rejects any registered waiter with the
    /// "killed by orchestrator" Crashed variant. The tool surfaces it
    /// as `{"error": "crashed", "details": "...killed by orchestrator"}`.
    #[tokio::test]
    async fn wait_agent_surfaces_kill_as_crashed() {
        let h = setup();
        h.bus
            .register_handle("exec-killed", ExecutionHandle::new(10));
        let bus_for_killer = h.bus.clone();
        let tool = WaitAgentTool::new(h.bus, h.state_service, h.messages);

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            bus_for_killer.kill("exec-killed");
        });

        let result = tool
            .execute(
                dummy_ctx(),
                json!({ "execution_id": "exec-killed", "timeout_secs": 5 }),
            )
            .await
            .unwrap();

        assert_eq!(result["error"], "crashed");
        assert!(result["details"]
            .as_str()
            .unwrap_or_default()
            .contains("killed by orchestrator"));
    }
}
