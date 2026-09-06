//! Live execution control, separate from invocation and persisted-subagent recovery.
//! The registry is shared with bootstrap/streaming; never construct a second map.

use crate::{DelegationRegistry, ExecutionHandle};
use execution_state::StateService;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use zbot_runtime_sqlite::DatabaseManager;

pub(super) struct SessionControl {
    pub(super) handles: Arc<RwLock<HashMap<String, ExecutionHandle>>>,
    pub(super) delegation_registry: Arc<DelegationRegistry>,
    pub(super) state_service: Arc<StateService<DatabaseManager>>,
}

impl SessionControl {
    pub(super) async fn stop(&self, conversation_id: &str) -> Result<(), String> {
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
            Err(format!(
                "No active execution for conversation: {}",
                conversation_id
            ))
        }
    }

    pub(super) async fn continue_execution(
        &self,
        conversation_id: &str,
        additional_iterations: u32,
    ) -> Result<(), String> {
        let handles = self.handles.read().await;
        if let Some(handle) = handles.get(conversation_id) {
            handle.add_iterations(additional_iterations);
            Ok(())
        } else {
            Err(format!(
                "No active execution for conversation: {}",
                conversation_id
            ))
        }
    }

    pub(super) async fn pause(&self, session_id: &str) -> Result<(), String> {
        // First update the database state
        self.state_service.pause_session(session_id)?;

        // Preserve the legacy broad handle signaling used by this entry point.
        let handles = self.handles.read().await;
        for handle in handles.values() {
            handle.pause();
        }

        Ok(())
    }

    pub(super) async fn cancel(&self, session_id: &str) -> Result<(), String> {
        // First update the database state
        self.state_service.cancel_session(session_id)?;

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
    ) -> Result<(), String> {
        self.state_service.cancel_session(session_id)?;

        cancel_execution_tree(
            &*self.handles.read().await,
            &self.delegation_registry,
            conversation_id,
        );

        Ok(())
    }

    pub(super) async fn end_session(&self, session_id: &str) -> Result<(), String> {
        tracing::info!(session_id = %session_id, "User requested session end");

        // Stop any running executions gracefully
        let handles = self.handles.read().await;
        for handle in handles.values() {
            handle.stop();
        }

        // Mark session as completed
        self.state_service.complete_session(session_id)?;

        tracing::info!(session_id = %session_id, "Session ended by user request");
        Ok(())
    }

    pub(super) async fn get_handle(&self, conversation_id: &str) -> Option<ExecutionHandle> {
        let handles = self.handles.read().await;
        handles.get(conversation_id).cloned()
    }

    pub(super) async fn resume_live(&self, session_id: &str) -> Result<(), String> {
        self.state_service.resume_session(session_id)?;

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
        assert_eq!(
            control.stop("missing").await.unwrap_err(),
            "No active execution for conversation: missing"
        );
        assert_eq!(
            control.continue_execution("missing", 5).await.unwrap_err(),
            "No active execution for conversation: missing"
        );
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
