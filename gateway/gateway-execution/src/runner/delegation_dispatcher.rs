//! # DelegationDispatcher
//!
//! Long-lived per-session queue for spawning subagents. Within a session,
//! delegations run sequentially. Across sessions, they interleave up to the
//! configured semaphore cap.
//!
//! ## Architecture
//!
//! `DelegationDispatcher` is a 3-field struct: it holds only the concurrency
//! semaphore, the inbound request channel, and a [`DelegationSpawner`]. Everything
//! else (the 21 deps that `spawn_delegated_agent` needs) lives inside the
//!
//! This keeps the dispatcher testable with a `StubSessionInvoker` (one trait
//! method per stub) while keeping the production path complete.
//!
//! ## Queue semantics
//!
//! - Sequential (`parallel: false`): only one delegation per session runs at a
//!   time; extras are queued and dispatched in order as each finishes.
//! - Parallel (`parallel: true`): skip the per-session queue and go straight
//!   to the global semaphore.
//! - Global cap: the `delegation_semaphore` gates total concurrent subagents
//!   regardless of session. The permit is acquired here and passed to the
//!   invoker so it holds it for the duration of the child execution.

use super::core::ExecutionRunner;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;

use crate::config::ExecutionConfig;
use crate::delegation::DelegationRequest;
use crate::runner::session_invoker::DelegationSpawner;
use gateway_events::GatewayEvent;
use serde_json::Value;

/// Dispatcher that enforces per-session sequential ordering and global
/// concurrency cap for subagent delegations.
pub struct DelegationDispatcher {
    pub delegation_rx: mpsc::UnboundedReceiver<DelegationRequest>,
    pub delegation_semaphore: Arc<Semaphore>,
    pub invoker: Arc<dyn DelegationSpawner>,
}

impl DelegationDispatcher {
    /// Start the dispatcher loop in a background task.
    ///
    /// Returns the `JoinHandle` — callers that only need fire-and-forget
    /// can drop it; tests hold it to `.await` shutdown.
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(mut self) {
        // Per-session tracking: only one delegation active per session at a time.
        let mut active_sessions: HashSet<String> = HashSet::new();
        let mut queued: HashMap<String, VecDeque<DelegationRequest>> = HashMap::new();

        // Completion notification channel: each spawned task sends its session_id
        // here when it finishes so the next queued request can be dispatched.
        let (done_tx, mut done_rx) = mpsc::unbounded_channel::<String>();

        // `rx_open` tracks whether the inbound request channel is still open.
        // When it closes the dispatcher drains in-flight work then exits.
        let mut rx_open = true;

        loop {
            tokio::select! {
                msg = self.delegation_rx.recv(), if rx_open => {
                    match msg {
                        Some(request) => {
                            let session_id = request.session_id.clone();

                            if request.parallel {
                                // Parallel: skip per-session queue, go straight to global semaphore.
                                tracing::info!(
                                    session_id = %session_id,
                                    child_agent = %request.child_agent_id,
                                    "Parallel delegation — bypassing per-session queue"
                                );
                                self.spawn_with_notification(request, done_tx.clone());
                            } else if active_sessions.contains(&session_id) {
                                // Sequential: queue behind the active delegation for this session.
                                tracing::info!(
                                    session_id = %session_id,
                                    agent = %request.child_agent_id,
                                    queued = queued.get(&session_id).map(|q| q.len()).unwrap_or(0),
                                    "Queuing delegation (active delegation in progress)"
                                );
                                queued.entry(session_id).or_default().push_back(request);
                            } else {
                                // Sequential: no active delegation, spawn immediately.
                                tracing::info!(
                                    session_id = %session_id,
                                    parent_agent = %request.parent_agent_id,
                                    child_agent = %request.child_agent_id,
                                    "Processing delegation request"
                                );
                                active_sessions.insert(session_id.clone());
                                self.spawn_with_notification(request, done_tx.clone());
                            }
                        }
                        None => {
                            // Inbound channel closed — stop accepting new requests.
                            rx_open = false;
                            tracing::info!("DelegationDispatcher: request channel closed, draining in-flight work");
                            // If nothing is in-flight, exit immediately.
                            if active_sessions.is_empty() && queued.is_empty() {
                                break;
                            }
                        }
                    }
                }
                Some(completed_session) = done_rx.recv() => {
                    active_sessions.remove(&completed_session);

                    // Pop the next queued request for this session (if any).
                    if let Some(queue) = queued.get_mut(&completed_session) {
                        if let Some(next) = queue.pop_front() {
                            tracing::info!(
                                session_id = %completed_session,
                                agent = %next.child_agent_id,
                                remaining = queue.len(),
                                "Dequeuing next delegation"
                            );
                            active_sessions.insert(completed_session.clone());
                            self.spawn_with_notification(next, done_tx.clone());
                        }
                        if queued
                            .get(&completed_session)
                            .map(|q| q.is_empty())
                            .unwrap_or(true)
                        {
                            queued.remove(&completed_session);
                        }
                    }

                    // If the inbound channel closed and all work is drained, exit.
                    if !rx_open && active_sessions.is_empty() && queued.is_empty() {
                        break;
                    }
                }
                else => break,
            }
        }
    }

    /// Acquire the global semaphore permit, call the invoker's
    /// `spawn_delegation`, then signal the run-loop via `done_tx` so the
    /// next queued request for the same session can be dispatched.
    ///
    /// The permit is passed *into* the invoker so it's held for the
    /// duration of the child execution (not just the spawn call).
    fn spawn_with_notification(
        &self,
        request: DelegationRequest,
        done_tx: mpsc::UnboundedSender<String>,
    ) {
        let session_id = request.session_id.clone();
        let semaphore = self.delegation_semaphore.clone();
        let invoker = self.invoker.clone();

        tokio::spawn(async move {
            let permit = semaphore.acquire_owned().await.ok();

            let child_agent_id = request.child_agent_id.clone();
            let result = invoker.spawn_delegation(request, permit).await;

            if let Err(e) = &result {
                tracing::error!(
                    session_id = %session_id,
                    agent = %child_agent_id,
                    error = %e,
                    "Delegation failed"
                );
            }

            // Notify the run-loop that this session's delegation is done.
            let _ = done_tx.send(session_id);
        });
    }
}

// ============================================================================
// ============================================================================

/// Per-ward serialization locks: ward name → an async mutex held for the
/// duration of that ward's currently-running ward-agent. The dispatcher
/// already serializes delegations per session; this closes the cross-session
/// gap, because a ward's shared files (`memory-bank/*.md`, specs) are written
/// by tools without filesystem locks.
pub(crate) type WardLocks = std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

/// Get-or-create the lock for `ward` and acquire it. The returned guard must
/// be held for the whole ward-agent execution.
///
/// The guard spans the entire child execution, including any sub-delegations
/// it issues. This assumes a ward-agent never delegates to another ward — the
/// ward-as-agent design routes sub-work to the generic worker agents, never to
/// sibling wards, so no `ward A → ward B → ward A` cycle (which would deadlock)
/// can form.
pub(super) async fn acquire_ward_lock(
    locks: &Arc<WardLocks>,
    ward: &str,
) -> tokio::sync::OwnedMutexGuard<()> {
    let ward_mutex = {
        let mut map = locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.entry(ward.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    ward_mutex.lock_owned().await
}

impl ExecutionRunner {
    /// Spawn a delegated subagent.
    ///
    /// This is called when an agent uses the delegate_to_agent tool.
    /// The subagent runs in a separate task with its own conversation.
    pub async fn spawn_delegation(
        &self,
        parent_agent_id: &str,
        parent_conversation_id: &str,
        child_agent_id: &str,
        task: &str,
        context: Option<Value>,
    ) -> Result<String, String> {
        // Generate child conversation ID
        let child_conversation_id = format!(
            "{}-sub-{}",
            parent_conversation_id,
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("0")
        );

        // Register the delegation (legacy function, using conversation_id as session for backward compat)
        let delegation_context = crate::delegation::DelegationContext::new(
            parent_conversation_id, // session_id (using conv_id for legacy)
            parent_conversation_id, // parent_execution_id (using conv_id for legacy)
            parent_agent_id,
            parent_conversation_id,
        );
        let delegation_context = if let Some(ctx) = context {
            delegation_context.with_context(ctx)
        } else {
            delegation_context
        };
        self.ctx
            .control
            .delegation_registry
            .register(&child_conversation_id, delegation_context);

        // Create config for the child agent
        let config = ExecutionConfig::new(
            child_agent_id.to_string(),
            child_conversation_id.clone(),
            self.ctx.paths.vault_dir().clone(),
        );

        // Emit delegation started event
        self.ctx
            .event_bus
            .publish(GatewayEvent::DelegationStarted {
                session_id: parent_conversation_id.to_string(), // legacy: using conv_id as session
                parent_execution_id: parent_conversation_id.to_string(),
                child_execution_id: child_conversation_id.clone(),
                parent_agent_id: parent_agent_id.to_string(),
                child_agent_id: child_agent_id.to_string(),
                task: task.to_string(),
                parent_conversation_id: Some(parent_conversation_id.to_string()),
                child_conversation_id: Some(child_conversation_id.clone()),
            })
            .await;

        // Spawn the child agent
        match self.invoke(config, task.to_string()).await {
            Ok((_handle, session_id)) => {
                tracing::info!(
                    parent_agent = %parent_agent_id,
                    child_agent = %child_agent_id,
                    child_conversation = %child_conversation_id,
                    session_id = %session_id,
                    "Spawned delegated subagent"
                );
                Ok(child_conversation_id)
            }
            Err(e) => {
                // Remove from registry on failure
                self.ctx
                    .control
                    .delegation_registry
                    .remove(&child_conversation_id);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ward_lock_serializes_same_ward() {
        let locks: Arc<WardLocks> = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let guard = acquire_ward_lock(&locks, "alpha").await;

        // A second acquire of the same ward must block while the guard is held.
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            acquire_ward_lock(&locks, "alpha"),
        )
        .await;
        assert!(blocked.is_err(), "same-ward lock must block while held");

        // After release the ward is acquirable again.
        drop(guard);
        let _reacquired = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            acquire_ward_lock(&locks, "alpha"),
        )
        .await
        .expect("ward lock must be free after release");
    }

    #[tokio::test]
    async fn ward_lock_allows_different_wards() {
        let locks: Arc<WardLocks> = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let _alpha = acquire_ward_lock(&locks, "alpha").await;

        // A different ward never contends with `alpha`.
        let beta = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            acquire_ward_lock(&locks, "beta"),
        )
        .await;
        assert!(beta.is_ok(), "different wards must not contend");
    }
}
