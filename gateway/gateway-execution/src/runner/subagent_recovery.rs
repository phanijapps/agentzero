//! Persisted subagent recovery: re-spawn crashed/paused children without
//! re-running the root. Owned separately from live session control.

use super::core::ExecutionRunner;
use crate::delegation::{spawn_delegated_agent, DelegationRequest};

impl ExecutionRunner {
    /// Re-spawn a crashed or gracefully paused subagent without re-running root.
    pub(super) async fn resume_persisted_subagent(
        &self,
        session_id: &str,
        crashed_exec: &execution_state::AgentExecution,
    ) -> Result<(), String> {
        let child_session_id = crashed_exec
            .child_session_id
            .as_ref()
            .ok_or("No child_session_id on crashed execution")?;

        // 1. Reactivate root session and execution.
        if self
            .control
            .state_service
            .get_session(session_id)?
            .is_some_and(|session| session.status == execution_state::SessionStatus::Paused)
        {
            self.control.state_service.resume_session(session_id)?;
        } else {
            self.control.state_service.reactivate_session(session_id)?;
        }
        if let Ok(Some(root_exec)) = self.control.state_service.get_root_execution(session_id) {
            self.control
                .state_service
                .reactivate_execution(&root_exec.id)?;
        }

        // 2. Preserve and reactivate the crashed execution identity. Durable
        // peer work is addressed to an execution ID; replacing that ID during
        // smart resume would orphan already-accepted messages.
        self.control
            .state_service
            .reactivate_execution(&crashed_exec.id)?;

        // 3. Reactivate the child session.
        if self
            .control
            .state_service
            .get_session(child_session_id)?
            .is_some_and(|session| session.status == execution_state::SessionStatus::Paused)
        {
            self.control
                .state_service
                .resume_session(child_session_id)?;
        } else {
            self.control
                .state_service
                .reactivate_session(child_session_id)?;
        }

        // 4. Ensure pending_delegations is at least 1 without double-counting
        // a gracefully paused delegation whose bookkeeping stayed durable.
        let parent_session = self
            .control
            .state_service
            .get_session(session_id)?
            .ok_or_else(|| format!("Session not found: {session_id}"))?;
        if !parent_session.has_pending_delegations() {
            self.control.state_service.register_delegation(session_id)?;
        }

        // 5. Request continuation so root agent processes the callback when subagent finishes
        self.control
            .state_service
            .request_continuation(session_id)?;

        // 6. Build DelegationRequest from crashed execution's data
        let parent_execution_id = crashed_exec
            .parent_execution_id
            .as_ref()
            .ok_or("No parent_execution_id on crashed execution")?;

        let task = crashed_exec
            .task
            .as_ref()
            .ok_or("No task on crashed execution")?;

        // Get root agent ID for parent_agent_id
        let root_agent_id = self
            .control
            .state_service
            .get_root_execution(session_id)?
            .map(|e| e.agent_id)
            .unwrap_or_else(|| "root".to_string());

        let request = DelegationRequest {
            parent_agent_id: root_agent_id,
            session_id: session_id.to_string(),
            parent_execution_id: parent_execution_id.clone(),
            // Resume-from-crash: parent's conversation_id is not separately tracked
            // here. The root agent's conversation_id equals session_id by convention,
            // so use session_id as a best-effort fallback. This is consistent with
            // the legacy emit at runner/core.rs spawn_delegation.
            parent_conversation_id: session_id.to_string(),
            child_agent_id: crashed_exec.agent_id.clone(),
            child_execution_id: crashed_exec.id.clone(),
            task: task.clone(),
            mode: None,
            context: None,
            max_iterations: None,
            output_schema: None,
            skills: vec![],
            capability_assignment: None,
            planning_capability_catalog: None,
            complexity: None,
            parallel: false,
        };

        // 7. Re-spawn the subagent
        let integrations = self.integrations.snapshot();
        spawn_delegated_agent(
            &request,
            self.event_bus.clone(),
            self.agent_service.clone(),
            self.provider_service.clone(),
            self.mcp_service.clone(),
            self.skill_service.clone(),
            self.paths.clone(),
            self.messages.clone(),
            self.session_meta.clone(),
            self.checkpoints.clone(),
            self.control.handles.clone(),
            self.control.delegation_registry.clone(),
            self.delegation_tx.clone(),
            self.log_service.clone(),
            self.control.state_service.clone(),
            None, // No delegation permit needed for resume
            self.memory_store.clone(),
            self.distiller.clone(),
            self.memory_recall.clone(),
            self.peer_messages.clone(),
            self.a2a_delegation.clone(),
            self.rate_limiters.clone(),
            integrations.kg_store,
            integrations.ingestion_adapter,
            integrations.goal_adapter,
            self.steering_registry.clone(),
            self.agent_result_bus.clone(),
        )
        .await?;

        Ok(())
    }
}
