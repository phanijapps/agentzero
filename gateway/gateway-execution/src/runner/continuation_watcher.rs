//! # ContinuationWatcher
//!
//! 2-field handler that listens for [`GatewayEvent::SessionContinuationReady`]
//! and invokes the session continuation path through [`ContinuationSpawner`].
//!
//! Extracted from the inline `spawn_continuation_handler` closure in
//! `ExecutionRunner::new` so that the event-loop contract can be tested
//! without wiring up the full runner pipeline.
//!
//! ## Spec deviations (intentional)
//! - Struct has 2 fields (`event_bus`, `invoker`), not 3. `state_service`
//!   was dropped because `clear_continuation` is called inside
//!   `ExecCtx::spawn_continuation`.
//! - `ExecCtx` implements the spawner directly; there is no companion
//!   cloned runner fields needed by `invoke_continuation`. It exists so
//!   the watcher can be wired inside `ExecutionRunner::with_config`
//!   without requiring `Arc<ExecutionRunner>` at construction time.

use super::session_invoker::ContinuationSpawner;
use gateway_events::{EventBus, GatewayEvent};
use std::sync::Arc;
use tokio::sync::broadcast;

// ============================================================================
// RunnerContinuationInvoker
// ============================================================================

// ============================================================================
// ContinuationWatcher
// ============================================================================

/// Listens for `SessionContinuationReady` events and invokes the continuation
/// path via the injected [`ContinuationSpawner`].
pub struct ContinuationWatcher {
    pub event_bus: Arc<EventBus>,
    pub invoker: Arc<dyn ContinuationSpawner>,
}

impl ContinuationWatcher {
    /// Start the watcher loop in a background task.
    ///
    /// Returns the `JoinHandle` — callers that only need fire-and-forget
    /// can drop it; tests hold it to `.await` shutdown.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        let mut event_rx = self.event_bus.subscribe_all();
        let invoker = self.invoker.clone();

        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(GatewayEvent::SessionContinuationReady {
                        session_id,
                        root_agent_id,
                        root_execution_id,
                    }) => {
                        tracing::info!(
                            session_id = %session_id,
                            root_agent_id = %root_agent_id,
                            root_execution_id = %root_execution_id,
                            "ContinuationWatcher: SessionContinuationReady received"
                        );
                        Self::handle(&*invoker, session_id, root_agent_id).await;
                    }
                    Ok(_) => {
                        // Ignore other events.
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("ContinuationWatcher: event bus lagged by {} events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("ContinuationWatcher: event bus closed, shutting down");
                        break;
                    }
                }
            }
        })
    }

    async fn handle(invoker: &dyn ContinuationSpawner, session_id: String, root_agent_id: String) {
        if let Err(error) = invoker
            .spawn_continuation(session_id.clone(), root_agent_id)
            .await
        {
            tracing::error!(
                session_id = %session_id,
                %error,
                "ContinuationWatcher: spawn_continuation failed"
            );
        }
    }
}
