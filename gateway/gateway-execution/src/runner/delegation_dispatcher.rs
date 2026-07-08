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
//! `RunnerDelegationInvoker` companion, which is constructed by
//! `ExecutionRunner::make_delegation_invoker()` and injected at wire-up time.
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

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use api_logs::LogService;
use async_trait::async_trait;
use execution_state::StateService;
use gateway_events::EventBus;
use gateway_services::{AgentService, McpService, ProviderService, SharedVaultPaths};
use tokio::sync::{mpsc, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::JoinHandle;
use zbot_stores_sqlite::DatabaseManager;

use crate::agent_pool::AgentResultBus;
use crate::delegation::{spawn_delegated_agent, DelegationRegistry, DelegationRequest};
use crate::handle::ExecutionHandle;
use crate::runner::session_invoker::DelegationSpawner;

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
// RunnerDelegationInvoker
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
async fn acquire_ward_lock(locks: &Arc<WardLocks>, ward: &str) -> tokio::sync::OwnedMutexGuard<()> {
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

/// Companion to `ExecutionRunner` that holds the subset of runner fields
/// needed to call `spawn_delegated_agent`, implementing [`DelegationSpawner`]
/// so `DelegationDispatcher` remains decoupled from the concrete runner type.
///
/// Constructed via [`ExecutionRunner::make_delegation_invoker`] inside
/// `with_config` — before the runner is wrapped in `Arc` — so each field
/// gets a clone of the runner's shared handles rather than ownership.
///
/// The full delegation pipeline runs inside `spawn_delegation`:
/// child session creation, delegation registry lifecycle, event emission,
/// subagent rules + ward context + recall priming, executor build + run,
/// success/failure callbacks, and continuation trigger.
pub(crate) struct RunnerDelegationInvoker {
    pub(crate) event_bus: Arc<EventBus>,
    pub(crate) agent_service: Arc<AgentService>,
    pub(crate) provider_service: Arc<ProviderService>,
    pub(crate) mcp_service: Arc<McpService>,
    pub(crate) skill_service: Arc<gateway_services::SkillService>,
    pub(crate) paths: SharedVaultPaths,
    pub(crate) messages: Arc<dyn zbot_conversation::MessageStore>,
    pub(crate) session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    pub(crate) checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    pub(crate) handles: Arc<RwLock<HashMap<String, ExecutionHandle>>>,
    pub(crate) delegation_registry: Arc<DelegationRegistry>,
    pub(crate) delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    pub(crate) log_service: Arc<LogService<DatabaseManager>>,
    pub(crate) state_service: Arc<StateService<DatabaseManager>>,
    pub(crate) memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    pub(crate) distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    pub(crate) memory_recall: Option<Arc<crate::recall::MemoryRecall>>,
    pub(crate) rate_limiters: Arc<
        std::sync::RwLock<
            std::collections::HashMap<String, Arc<agent_runtime::ProviderRateLimiter>>,
        >,
    >,
    pub(crate) kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,
    pub(crate) ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>>,
    pub(crate) goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>>,
    pub(crate) steering_registry: Arc<agent_runtime::SteeringRegistry>,
    pub(crate) agent_result_bus: Arc<AgentResultBus>,
    /// Per-ward serialization locks (see [`acquire_ward_lock`]).
    pub(crate) ward_locks: Arc<WardLocks>,
    /// Per-ward usage telemetry — bumped on every `ward:<name>` delegation.
    pub(crate) ward_usage: Arc<gateway_services::WardUsage>,
}

#[async_trait]
impl DelegationSpawner for RunnerDelegationInvoker {
    async fn spawn_delegation(
        &self,
        request: DelegationRequest,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Result<(), String> {
        // Bump per-ward usage telemetry as soon as we know this is a ward
        // delegation, before any locking. The curator reads these counters
        // to decide what's active vs stale (see Phase B). Failures here
        // never block delegation — telemetry is best-effort.
        let ward_name = request.child_agent_id.strip_prefix("ward:");
        if let Some(ward) = ward_name {
            if let Err(e) = self.ward_usage.bump_use(ward) {
                tracing::warn!(ward = %ward, error = %e, "ward_usage.bump_use failed");
            }
        }

        // Serialize ward-agent delegations per ward. Ward-shared files
        // (memory-bank/*.md, specs) are written by tools without filesystem
        // locks; the dispatcher only serializes per session, so two sessions
        // delegating to the same ward could lose updates. Holding this guard
        // for the whole child execution makes it one ward-agent per ward.
        let _ward_guard = match ward_name {
            Some(ward) => Some(acquire_ward_lock(&self.ward_locks, ward).await),
            None => None,
        };
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
            self.handles.clone(),
            self.delegation_registry.clone(),
            self.delegation_tx.clone(),
            self.log_service.clone(),
            self.state_service.clone(),
            permit,
            self.memory_store.clone(),
            self.distiller.clone(),
            self.memory_recall.clone(),
            self.rate_limiters.clone(),
            self.kg_store.clone(),
            self.ingestion_adapter.clone(),
            self.goal_adapter.clone(),
            self.steering_registry.clone(),
            self.agent_result_bus.clone(),
        )
        .await
        .map(|_| ())
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
