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
//!   `RunnerContinuationInvoker::spawn_continuation`.
//! - `RunnerContinuationInvoker` is a private companion that holds the
//!   cloned runner fields needed by `invoke_continuation`. It exists so
//!   the watcher can be wired inside `ExecutionRunner::with_config`
//!   without requiring `Arc<ExecutionRunner>` at construction time.

use super::continuation_execution::{invoke_continuation, ContinuationArgs};
use super::core::ExecutionRunner;
use super::session_invoker::ContinuationSpawner;
use api_logs::LogService;
use async_trait::async_trait;
use execution_state::StateService;
use gateway_events::{EventBus, GatewayEvent};
use gateway_services::{AgentService, McpService, ProviderService, SharedVaultPaths};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use zbot_runtime_sqlite::DatabaseManager;

use crate::delegation::{DelegationRegistry, DelegationRequest};
use crate::handle::ExecutionHandle;

// ============================================================================
// RunnerContinuationInvoker
// ============================================================================

/// Companion to `ExecutionRunner` that holds the subset of runner fields
/// needed to call `invoke_continuation`, implementing [`ContinuationSpawner`]
/// so `ContinuationWatcher` remains decoupled from the concrete runner type.
///
/// Constructed via [`ExecutionRunner::make_continuation_invoker`] inside
/// `with_config` — before the runner is wrapped in `Arc` — so that each
/// field gets a clone of the runner's shared handles rather than ownership.
///
/// The critical `model_registry` field is stored as the
/// `Arc<ArcSwapOption<…>>` handle (not the inner value) so late calls to
/// `set_model_registry` are visible at fire time, preserving the fix for
/// the capture-before-init bug.
pub(crate) struct RunnerContinuationInvoker {
    pub(crate) event_bus: Arc<EventBus>,
    pub(crate) agent_service: Arc<AgentService>,
    pub(crate) provider_service: Arc<ProviderService>,
    pub(crate) mcp_service: Arc<McpService>,
    pub(crate) skill_service: Arc<gateway_services::SkillService>,
    pub(crate) paths: SharedVaultPaths,
    pub(crate) handles: Arc<RwLock<HashMap<String, ExecutionHandle>>>,
    pub(crate) messages: Arc<dyn zbot_conversation::MessageStore>,
    pub(crate) checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    pub(crate) delegation_registry: Arc<DelegationRegistry>,
    pub(crate) delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    pub(crate) log_service: Arc<LogService<DatabaseManager>>,
    pub(crate) state_service: Arc<StateService<DatabaseManager>>,
    pub(crate) memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    pub(crate) embedding_client: Option<Arc<dyn agent_runtime::llm::embedding::EmbeddingClient>>,
    pub(crate) distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    pub(crate) handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
    pub(crate) memory_recall: Option<Arc<crate::recall::MemoryRecall>>,
    pub(crate) peer_messages: Option<Arc<crate::peer_messaging::DurablePeerMessageService>>,
    pub(crate) a2a_delegation: Option<Arc<dyn crate::a2a::A2aDelegationService>>,
    pub(crate) steering_registry: Arc<agent_runtime::SteeringRegistry>,
    /// ArcSwap handle — NOT the inner `Option<Arc<ModelRegistry>>`. Reads
    /// the live value at fire time via `.load_full()`.
    pub(crate) model_registry:
        Arc<arc_swap::ArcSwapOption<gateway_services::models::ModelRegistry>>,
    pub(super) integrations: super::integrations::SharedIntegrations,
    pub(crate) procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    /// Per-ward usage telemetry — passed through to `invoke_continuation`
    /// so the ward tool's create action can mark new wards as agent-authored.
    pub(crate) ward_usage: Arc<gateway_services::WardUsage>,
}

#[async_trait]
impl ContinuationSpawner for RunnerContinuationInvoker {
    async fn spawn_continuation(
        &self,
        session_id: String,
        root_agent_id: String,
    ) -> Result<(), String> {
        if let Err(error) = self.state_service.clear_continuation(&session_id) {
            tracing::warn!(%session_id, %error, "Failed to clear continuation flag");
        }

        let integrations = self.integrations.snapshot();
        let result = invoke_continuation(ContinuationArgs {
            session_id: &session_id,
            root_agent_id: &root_agent_id,
            event_bus: self.event_bus.clone(),
            agent_service: self.agent_service.clone(),
            provider_service: self.provider_service.clone(),
            mcp_service: self.mcp_service.clone(),
            skill_service: self.skill_service.clone(),
            paths: self.paths.clone(),
            messages: self.messages.clone(),
            checkpoints: self.checkpoints.clone(),
            handles: self.handles.clone(),
            delegation_registry: self.delegation_registry.clone(),
            delegation_tx: self.delegation_tx.clone(),
            log_service: self.log_service.clone(),
            state_service: self.state_service.clone(),
            memory_store: self.memory_store.clone(),
            embedding_client: self.embedding_client.clone(),
            distiller: self.distiller.clone(),
            handoff_writer: self.handoff_writer.clone(),
            memory_recall: self.memory_recall.clone(),
            peer_messages: self.peer_messages.clone(),
            a2a_delegation: self.a2a_delegation.clone(),
            steering_registry: self.steering_registry.clone(),
            // Read the live registry at fire time — not a stale capture.
            model_registry: self.model_registry.load_full(),
            kg_store: integrations.kg_store,
            kg_episode_store: integrations.kg_episode_store,
            ingestion_adapter: integrations.ingestion_adapter,
            goal_adapter: integrations.goal_adapter,
            procedure_store: self.procedure_store.clone(),
            ward_usage: self.ward_usage.clone(),
        })
        .await;
        if let Err(error) = result {
            // An unstarted continuation must not leave the session hanging
            // with completed delegations and no terminal outcome: publish the
            // crash lifecycle so state and UI converge on a failed session.
            tracing::error!(
                session_id = %session_id,
                root_agent_id = %root_agent_id,
                %error,
                "ContinuationWatcher: spawn_continuation failed; crashing session"
            );
            let execution_id = self
                .state_service
                .get_root_execution(&session_id)
                .ok()
                .flatten()
                .map(|execution| execution.id)
                .unwrap_or_default();
            crate::lifecycle::crash_execution(crate::lifecycle::CrashExecution {
                state_service: &self.state_service,
                log_service: &self.log_service,
                event_bus: &self.event_bus,
                execution_id: &execution_id,
                session_id: &session_id,
                agent_id: &root_agent_id,
                conversation_id: &session_id,
                error: &error,
                crash_session: true,
            })
            .await;
        }
        Ok(())
    }
}

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

impl ExecutionRunner {
    /// Build a [`RunnerContinuationInvoker`] from this runner's fields.
    ///
    /// Called from `with_config` to wire the `ContinuationWatcher` before
    /// the runner is wrapped in `Arc`. Each field is cloned — the
    /// `model_registry` ArcSwap handle is cloned (not its inner value)
    /// so late-stored registries are visible at fire time.
    pub(super) fn make_continuation_invoker(
        &self,
    ) -> super::continuation_watcher::RunnerContinuationInvoker {
        super::continuation_watcher::RunnerContinuationInvoker {
            event_bus: self.event_bus.clone(),
            agent_service: self.agent_service.clone(),
            provider_service: self.provider_service.clone(),
            mcp_service: self.mcp_service.clone(),
            skill_service: self.skill_service.clone(),
            paths: self.paths.clone(),
            handles: self.control.handles.clone(),
            messages: self.messages.clone(),
            checkpoints: self.checkpoints.clone(),
            delegation_registry: self.control.delegation_registry.clone(),
            delegation_tx: self.delegation_tx.clone(),
            log_service: self.log_service.clone(),
            state_service: self.control.state_service.clone(),
            memory_store: self.memory_store.clone(),
            embedding_client: self.embedding_client.clone(),
            distiller: self.distiller.clone(),
            handoff_writer: self.handoff_writer.clone(),
            memory_recall: self.memory_recall.clone(),
            peer_messages: self.peer_messages.clone(),
            a2a_delegation: self.a2a_delegation.clone(),
            steering_registry: self.steering_registry.clone(),
            model_registry: self.model_registry.clone(),
            integrations: self.integrations.clone(),
            procedure_store: self.procedure_store.clone(),
            ward_usage: self.ward_usage.clone(),
        }
    }
}
