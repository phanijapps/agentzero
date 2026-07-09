use agent_primitives::{AgentError, Result, Tool, ToolContext};
use async_trait::async_trait;
use execution_state::{DelegationType, ExecutionFilter, StateService};
use serde_json::{json, Value};
use std::sync::Arc;
use zbot_stores_sqlite::DatabaseManager;

pub struct ListSessionAgentsTool {
    state_service: Arc<StateService<DatabaseManager>>,
}

impl ListSessionAgentsTool {
    pub fn new(state_service: Arc<StateService<DatabaseManager>>) -> Self {
        Self { state_service }
    }
}

#[async_trait]
impl Tool for ListSessionAgentsTool {
    fn name(&self) -> &'static str {
        "list_session_agents"
    }

    fn description(&self) -> &'static str {
        "List delegated agents in the current session. \
         Returns execution_id, agent_id, status, task, timestamps, and child_session_id. \
         Use this before handoff_to_agent when you need the execution_id for a running child."
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }))
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, _args: Value) -> Result<Value> {
        let session_id = context_session_id(ctx.as_ref())?;
        let executions = self
            .state_service
            .list_executions(&ExecutionFilter {
                session_id: Some(session_id.clone()),
                ..Default::default()
            })
            .map_err(|e| AgentError::Tool(format!("failed to list session agents: {e}")))?;

        let agents: Vec<Value> = executions
            .into_iter()
            .filter(|exec| exec.delegation_type != DelegationType::Root)
            .map(|exec| {
                json!({
                    "execution_id": exec.id,
                    "agent_id": exec.agent_id,
                    "status": exec.status.as_str(),
                    "task": exec.task,
                    "started_at": exec.started_at,
                    "completed_at": exec.completed_at,
                    "child_session_id": exec.child_session_id,
                })
            })
            .collect();

        Ok(json!({
            "session_id": session_id,
            "agents": agents,
        }))
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
    use execution_state::StateService;
    use gateway_services::VaultPaths;
    use serde_json::json;
    use std::collections::HashMap;
    use tempfile::TempDir;

    struct Harness {
        _tmp: TempDir,
        state_service: Arc<StateService<DatabaseManager>>,
    }

    fn setup() -> Harness {
        let tmp = TempDir::new().expect("tempdir");
        let paths = Arc::new(VaultPaths::new(tmp.path().to_path_buf()));
        paths.ensure_dirs_exist().expect("ensure vault dirs");
        let db = Arc::new(DatabaseManager::new(paths).expect("db init"));
        let state_service = Arc::new(StateService::new(db));
        Harness {
            _tmp: tmp,
            state_service,
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

    #[tokio::test]
    async fn list_session_agents_returns_only_current_session_delegations() {
        let h = setup();
        let (session, root) = h.state_service.create_session("root").unwrap();
        let (other_session, other_root) = h.state_service.create_session("root").unwrap();
        let child = h
            .state_service
            .create_delegated_execution(
                &session.id,
                "research-agent",
                &root.id,
                execution_state::DelegationType::Parallel,
                "research sources",
            )
            .unwrap();
        h.state_service.start_execution(&child.id).unwrap();
        let child_session = execution_state::Session::new_child("research-agent", &session.id);
        let child_session_id = child_session.id.clone();
        h.state_service
            .create_session_from(&child_session)
            .expect("create child session");
        h.state_service
            .set_child_session_id(&child.id, &child_session_id)
            .unwrap();
        h.state_service
            .create_delegated_execution(
                &other_session.id,
                "builder-agent",
                &other_root.id,
                execution_state::DelegationType::Parallel,
                "build thing",
            )
            .unwrap();

        let tool = ListSessionAgentsTool::new(h.state_service);
        let result = tool
            .execute(ctx_with_session(&session.id), json!({}))
            .await
            .unwrap();

        assert_eq!(result["session_id"], session.id);
        let agents = result["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["execution_id"], child.id);
        assert_eq!(agents[0]["agent_id"], "research-agent");
        assert_eq!(agents[0]["status"], "running");
        assert_eq!(agents[0]["task"], "research sources");
        assert!(agents[0]["started_at"].is_string());
        assert!(agents[0]["completed_at"].is_null());
        assert_eq!(agents[0]["child_session_id"], child_session_id);
    }

    #[tokio::test]
    async fn list_session_agents_requires_context_session_id() {
        let h = setup();
        let tool = ListSessionAgentsTool::new(h.state_service);
        let err = tool
            .execute(ctx_without_session(), json!({}))
            .await
            .unwrap_err();

        assert!(format!("{err}").contains("session_id"));
    }
}
