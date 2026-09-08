//! Live execution control, separate from invocation and persisted-subagent recovery.
//! The registry is shared with bootstrap/streaming; never construct a second map.

use crate::errors::ExecutionError;
use crate::{DelegationRegistry, ExecutionHandle};
use execution_state::StateService;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use zbot_runtime_sqlite::DatabaseManager;

#[derive(Clone)]
pub struct SessionControl {
    pub(crate) handles: Arc<RwLock<HashMap<String, ExecutionHandle>>>,
    pub(crate) delegation_registry: Arc<DelegationRegistry>,
    pub(crate) state_service: Arc<StateService<DatabaseManager>>,
}

impl SessionControl {
    pub(super) async fn stop(&self, conversation_id: &str) -> Result<(), ExecutionError> {
        let handles = self.handles.read().await;
        let stopped_root = handles.get(conversation_id).is_some();
        if let Some(handle) = handles.get(conversation_id) {
            handle.stop();
        }
        for child_conv_id in self.delegation_registry.get_children(conversation_id) {
            if let Some(child) = handles.get(&child_conv_id) {
                child.stop();
                tracing::info!(
                    parent = %conversation_id,
                    child = %child_conv_id,
                    "Cascaded stop signal to delegated subagent"
                );
            }
        }
        if stopped_root {
            Ok(())
        } else {
            Err(ExecutionError::from(format!(
                "No active execution for conversation: {}",
                conversation_id
            )))
        }
    }

    pub(super) async fn continue_execution(
        &self,
        conversation_id: &str,
        additional_iterations: u32,
    ) -> Result<(), ExecutionError> {
        let handles = self.handles.read().await;
        if let Some(handle) = handles.get(conversation_id) {
            handle.add_iterations(additional_iterations);
            Ok(())
        } else {
            Err(ExecutionError::from(format!(
                "No active execution for conversation: {}",
                conversation_id
            )))
        }
    }

    pub(super) async fn pause(&self, session_id: &str) -> Result<(), ExecutionError> {
        // First update the database state
        self.state_service
            .pause_session(session_id)
            .map_err(ExecutionError::from)?;

        // Preserve the legacy broad handle signaling used by this entry point.
        let handles = self.handles.read().await;
        for handle in handles.values() {
            handle.pause();
        }

        Ok(())
    }

    pub(super) async fn cancel(&self, session_id: &str) -> Result<(), ExecutionError> {
        // First update the database state
        self.state_service
            .cancel_session(session_id)
            .map_err(ExecutionError::from)?;

        // Then cancel any running execution
        let handles = self.handles.read().await;
        for handle in handles.values() {
            handle.cancel();
        }

        Ok(())
    }

    pub(super) async fn cancel_exact(
        &self,
        session_id: &str,
        conversation_id: &str,
    ) -> Result<(), ExecutionError> {
        self.state_service
            .cancel_session(session_id)
            .map_err(ExecutionError::from)?;

        cancel_execution_tree(
            &*self.handles.read().await,
            &self.delegation_registry,
            conversation_id,
        );

        Ok(())
    }

    pub(super) async fn end_session(&self, session_id: &str) -> Result<(), ExecutionError> {
        tracing::info!(session_id = %session_id, "User requested session end");

        // Stop any running executions gracefully
        let handles = self.handles.read().await;
        for handle in handles.values() {
            handle.stop();
        }

        // Mark session as completed
        self.state_service
            .complete_session(session_id)
            .map_err(ExecutionError::from)?;

        tracing::info!(session_id = %session_id, "Session ended by user request");
        Ok(())
    }

    pub(super) async fn get_handle(&self, conversation_id: &str) -> Option<ExecutionHandle> {
        let handles = self.handles.read().await;
        handles.get(conversation_id).cloned()
    }

    pub(super) async fn resume_live(&self, session_id: &str) -> Result<(), ExecutionError> {
        self.state_service
            .resume_session(session_id)
            .map_err(ExecutionError::from)?;

        let handles = self.handles.read().await;
        for handle in handles.values() {
            handle.resume();
        }

        Ok(())
    }
}

fn cancel_execution_tree(
    handles: &HashMap<String, ExecutionHandle>,
    delegations: &DelegationRegistry,
    root_conversation_id: &str,
) {
    let mut pending = vec![root_conversation_id.to_owned()];
    let mut visited = std::collections::HashSet::new();

    while let Some(conversation_id) = pending.pop() {
        if !visited.insert(conversation_id.clone()) {
            continue;
        }
        if let Some(handle) = handles.get(&conversation_id) {
            handle.cancel();
        }
        pending.extend(delegations.get_children(&conversation_id));
    }
}

use super::core::ExecutionRunner;

impl ExecutionRunner {
    /// Stop an execution by conversation ID.
    ///
    /// Cascades the stop signal to any delegated subagents currently
    /// running under this conversation. Without the cascade, stopping a
    /// root that's awaiting a planner would only signal the root's
    /// handle; the planner would keep running until its next iteration
    /// boundary. The cascade is one-level (parent → direct children) —
    /// extend to a BFS over `get_children` if multi-level delegation
    /// becomes common.
    pub async fn stop(&self, conversation_id: &str) -> Result<(), ExecutionError> {
        self.ctx.control.stop(conversation_id).await
    }

    /// Continue an execution after max iterations.
    pub async fn continue_execution(
        &self,
        conversation_id: &str,
        additional_iterations: u32,
    ) -> Result<(), ExecutionError> {
        self.ctx
            .control
            .continue_execution(conversation_id, additional_iterations)
            .await
    }

    /// Pause an execution by session ID.
    ///
    /// Pausing sets a flag that the executor will check. The execution
    /// will complete the current operation and then wait for resume.
    pub async fn pause(&self, session_id: &str) -> Result<(), ExecutionError> {
        self.ctx.control.pause(session_id).await
    }

    /// Resume a paused or crashed execution by session ID.
    ///
    /// For crashed sessions with a crashed subagent: re-spawns only the crashed
    /// subagent using its child session's message history, avoiding root re-evaluation.
    /// For paused sessions or root-only crashes: falls through to current behavior.
    pub async fn resume(&self, session_id: &str) -> Result<(), ExecutionError> {
        // Rebuild a persisted delegated execution before falling back to live
        // handles. After either a crash or graceful daemon shutdown there are
        // no in-memory handles to wake, and durable peer work still targets the
        // original execution ID.
        let resumable_subagent = match self
            .ctx
            .control
            .state_service
            .get_last_crashed_subagent(session_id)?
        {
            some @ Some(_) => some,
            None => self
                .ctx
                .control
                .state_service
                .list_executions(&execution_state::ExecutionFilter {
                    session_id: Some(session_id.to_owned()),
                    status: Some(execution_state::ExecutionStatus::Paused),
                    ..Default::default()
                })?
                .into_iter()
                .find(|execution| {
                    execution.parent_execution_id.is_some() && execution.child_session_id.is_some()
                }),
        };
        if let Some(resumable_exec) = resumable_subagent {
            if resumable_exec.child_session_id.is_some() {
                tracing::info!(
                    session_id = %session_id,
                    resumed_agent = %resumable_exec.agent_id,
                    prior_status = %resumable_exec.status.as_str(),
                    child_session = ?resumable_exec.child_session_id,
                    "Smart resume: re-spawning persisted subagent instead of root"
                );
                return self
                    .resume_persisted_subagent(session_id, &resumable_exec)
                    .await;
            }
        }

        // Fallback: standard resume (paused sessions or root-only crashes)
        self.ctx.control.resume_live(session_id).await
    }

    /// Cancel an execution by session ID.
    ///
    /// Cancellation immediately stops the execution and marks it as cancelled.
    pub async fn cancel(&self, session_id: &str) -> Result<(), ExecutionError> {
        self.ctx.control.cancel(session_id).await
    }

    /// Cancel one session and signal its exact conversation's delegation tree.
    /// This is used by externally scoped work where signaling
    /// unrelated executions would cross an authorization boundary.
    pub async fn cancel_exact(
        &self,
        session_id: &str,
        conversation_id: &str,
    ) -> Result<(), ExecutionError> {
        self.ctx
            .control
            .cancel_exact(session_id, conversation_id)
            .await
    }

    /// End a session (mark as completed).
    ///
    /// Called when user explicitly ends a session via /end, /new, or +new button.
    /// This marks the session as completed regardless of running executions.
    pub async fn end_session(&self, session_id: &str) -> Result<(), ExecutionError> {
        self.ctx.control.end_session(session_id).await
    }

    /// Get execution handle for a conversation.
    pub async fn get_handle(&self, conversation_id: &str) -> Option<ExecutionHandle> {
        self.ctx.control.get_handle(conversation_id).await
    }

    /// Get the delegation registry.
    pub fn delegation_registry(&self) -> Arc<DelegationRegistry> {
        self.ctx.control.delegation_registry.clone()
    }

    /// Get the state service for execution state management.
    pub fn state_service(&self) -> Arc<StateService<DatabaseManager>> {
        self.ctx.control.state_service.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> (tempfile::TempDir, SessionControl) {
        let temp = tempfile::tempdir().unwrap();
        let paths = Arc::new(gateway_services::VaultPaths::new(temp.path().to_path_buf()));
        paths.ensure_dirs_exist().unwrap();
        let db = Arc::new(DatabaseManager::new(paths).unwrap());
        let control = SessionControl {
            handles: Arc::new(RwLock::new(HashMap::new())),
            delegation_registry: Arc::new(DelegationRegistry::new()),
            state_service: Arc::new(StateService::new(db)),
        };
        (temp, control)
    }

    #[tokio::test]
    async fn lookup_shares_live_handle_and_extension_clears_stop() {
        let (_temp, control) = control();
        let handle = ExecutionHandle::new(10);
        control
            .handles
            .write()
            .await
            .insert("root".into(), handle.clone());
        control.get_handle("root").await.unwrap().stop();
        assert!(handle.is_stop_requested());
        control.continue_execution("root", 5).await.unwrap();
        assert_eq!(handle.max_iterations(), 15);
        assert!(!handle.is_stop_requested());
        assert!(control.get_handle("missing").await.is_none());
        assert!(control
            .stop("missing")
            .await
            .unwrap_err()
            .to_string()
            .contains("No active execution"));
        assert!(control
            .continue_execution("missing", 5)
            .await
            .unwrap_err()
            .to_string()
            .contains("No active execution"));
    }

    #[tokio::test]
    async fn stop_preserves_direct_child_scope() {
        let (_temp, control) = control();
        let root = ExecutionHandle::new(10);
        let child = ExecutionHandle::new(10);
        let grandchild = ExecutionHandle::new(10);
        *control.handles.write().await = HashMap::from([
            ("root".into(), root.clone()),
            ("child".into(), child.clone()),
            ("grandchild".into(), grandchild.clone()),
        ]);
        control.delegation_registry.register(
            "child",
            crate::DelegationContext::new("s", "r", "root", "root"),
        );
        control.delegation_registry.register(
            "grandchild",
            crate::DelegationContext::new("s", "c", "child", "child"),
        );
        control.stop("root").await.unwrap();
        assert!(root.is_stop_requested());
        assert!(child.is_stop_requested());
        assert!(!grandchild.is_stop_requested());
    }

    #[tokio::test]
    async fn pause_and_live_resume_persist_status_and_signal_shared_handles() {
        let (_temp, control) = control();
        let (session, _) = control.state_service.create_session("root").unwrap();
        let first = ExecutionHandle::new(10);
        let second = ExecutionHandle::new(10);
        *control.handles.write().await = HashMap::from([
            ("first".into(), first.clone()),
            ("second".into(), second.clone()),
        ]);
        control.pause(&session.id).await.unwrap();
        assert_eq!(
            control
                .state_service
                .get_session(&session.id)
                .unwrap()
                .unwrap()
                .status,
            execution_state::SessionStatus::Paused
        );
        assert!(first.is_paused() && second.is_paused());
        control.resume_live(&session.id).await.unwrap();
        assert_eq!(
            control
                .state_service
                .get_session(&session.id)
                .unwrap()
                .unwrap()
                .status,
            execution_state::SessionStatus::Running
        );
        assert!(!first.is_paused() && !second.is_paused());
    }

    #[tokio::test]
    async fn database_rejections_do_not_signal_handles() {
        let (_temp, control) = control();
        let handle = ExecutionHandle::new(10);
        control
            .handles
            .write()
            .await
            .insert("root".into(), handle.clone());
        assert!(control.pause("missing").await.is_err());
        assert!(!handle.is_paused());
        handle.pause();
        assert!(control.resume_live("missing").await.is_err());
        assert!(handle.is_paused());
        assert!(control.cancel("missing").await.is_err());
        assert!(control.cancel_exact("missing", "root").await.is_err());
        assert!(!handle.is_cancelled());
        assert!(!handle.is_stop_requested());
    }

    #[tokio::test]
    async fn cancel_and_end_preserve_legacy_broad_signaling() {
        let (_temp, control) = control();
        let (session, execution) = control.state_service.create_session("root").unwrap();
        let first = ExecutionHandle::new(10);
        let second = ExecutionHandle::new(10);
        *control.handles.write().await = HashMap::from([
            ("first".into(), first.clone()),
            ("second".into(), second.clone()),
        ]);
        control.cancel(&session.id).await.unwrap();
        assert!(first.is_cancelled() && second.is_cancelled());
        assert_eq!(
            control
                .state_service
                .get_execution(&execution.id)
                .unwrap()
                .unwrap()
                .status,
            execution_state::ExecutionStatus::Cancelled
        );
        let (next, _) = control.state_service.create_session("root").unwrap();
        let active = ExecutionHandle::new(10);
        control
            .handles
            .write()
            .await
            .insert("next".into(), active.clone());
        control.end_session(&next.id).await.unwrap();
        assert!(active.is_stop_requested());
        assert!(!active.is_cancelled());
        assert_eq!(
            control
                .state_service
                .get_session(&next.id)
                .unwrap()
                .unwrap()
                .status,
            execution_state::SessionStatus::Completed
        );
    }

    #[tokio::test]
    async fn exact_cancel_persists_and_isolates_the_selected_tree() {
        let (_temp, control) = control();
        let (session, execution) = control.state_service.create_session("root").unwrap();
        let root = ExecutionHandle::new(10);
        let child = ExecutionHandle::new(10);
        let other = ExecutionHandle::new(10);
        *control.handles.write().await = HashMap::from([
            ("root".into(), root.clone()),
            ("child".into(), child.clone()),
            ("other".into(), other.clone()),
        ]);
        control.delegation_registry.register(
            "child",
            crate::DelegationContext::new(&session.id, &execution.id, "root", "root"),
        );
        control.cancel_exact(&session.id, "root").await.unwrap();
        assert!(root.is_cancelled() && child.is_cancelled());
        assert!(!other.is_cancelled());
        assert_eq!(
            control
                .state_service
                .get_execution(&execution.id)
                .unwrap()
                .unwrap()
                .status,
            execution_state::ExecutionStatus::Cancelled
        );
    }

    #[test]
    fn exact_cancel_does_not_signal_an_unrelated_execution() {
        let selected = ExecutionHandle::new(10);
        let unrelated = ExecutionHandle::new(10);
        let handles = HashMap::from([
            ("selected".to_string(), selected.clone()),
            ("unrelated".to_string(), unrelated.clone()),
        ]);

        cancel_execution_tree(&handles, &DelegationRegistry::new(), "selected");

        assert!(selected.is_cancelled());
        assert!(!unrelated.is_cancelled());
    }
    #[test]
    fn session_stop_recursive_cancellation_contract() {
        let root = ExecutionHandle::new(10);
        let child = ExecutionHandle::new(10);
        let grandchild = ExecutionHandle::new(10);
        let unrelated = ExecutionHandle::new(10);
        let handles = HashMap::from([
            ("root".to_owned(), root.clone()),
            ("child".to_owned(), child.clone()),
            ("grandchild".to_owned(), grandchild.clone()),
            ("unrelated".to_owned(), unrelated.clone()),
        ]);

        let registry = DelegationRegistry::new();
        registry.register(
            "child",
            crate::delegation::DelegationContext::new("session", "root", "root", "root"),
        );
        registry.register(
            "grandchild",
            crate::delegation::DelegationContext::new("session", "child", "child", "child"),
        );
        cancel_execution_tree(&handles, &registry, "root");

        assert!(root.is_cancelled());
        assert!(child.is_cancelled(), "all descendants must be cancelled");
        assert!(
            grandchild.is_cancelled(),
            "all descendants must be cancelled"
        );
        assert!(!unrelated.is_cancelled());
    }
}
