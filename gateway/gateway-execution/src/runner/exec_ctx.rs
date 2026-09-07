//! ExecCtx — the set of services an execution needs, built once and shared
//! by `Arc`. Every entry path (root invoke, continuation, delegation,
//! recovery) operates on the same context; there is exactly one statement
//! of the dependency set.

use super::integrations::SharedIntegrations;
use super::session_control::SessionControl;
use crate::agent_pool::AgentResultBus;
use crate::delegation::DelegationRequest;
use crate::errors::ExecutionError;
use api_logs::LogService;
use execution_state::StateService;
use gateway_events::EventBus;
use gateway_services::{AgentService, McpService, ProviderService, SharedVaultPaths};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, Semaphore};

/// Shared execution context. Constructed once in
/// [`ExecutionRunner::with_config`](super::ExecutionRunner::with_config);
/// every orchestration path receives `&ExecCtx` or `Arc<ExecCtx>`.
#[derive(Clone)]
pub(crate) struct ExecCtx {
    // --- Core services ---
    pub event_bus: Arc<EventBus>,
    pub agent_service: Arc<AgentService>,
    pub provider_service: Arc<ProviderService>,
    pub mcp_service: Arc<McpService>,
    pub skill_service: Arc<gateway_services::SkillService>,
    pub paths: SharedVaultPaths,
    pub log_service: Arc<LogService<zbot_runtime_sqlite::DatabaseManager>>,
    pub state_service: Arc<StateService<zbot_runtime_sqlite::DatabaseManager>>,

    // --- Conversation persistence ---
    pub messages: Arc<dyn zbot_conversation::MessageStore>,
    pub session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    pub checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,

    // --- Live control (handles, delegation registry) ---
    pub control: SessionControl,
    pub delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    /// Limits concurrent delegation spawns.
    pub delegation_semaphore: Arc<Semaphore>,

    // --- Optional integrations ---
    pub connector_registry: Option<Arc<gateway_connectors::ConnectorRegistry>>,
    pub bridge_registry: Option<Arc<gateway_bridge::BridgeRegistry>>,
    pub bridge_outbox: Option<Arc<gateway_bridge::OutboxRepository>>,
    pub memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    pub embedding_client: Option<Arc<dyn agent_runtime::llm::embedding::EmbeddingClient>>,
    pub distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    pub handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
    pub memory_recall: Option<Arc<crate::recall::MemoryRecall>>,
    pub peer_messages: Option<Arc<crate::peer_messaging::DurablePeerMessageService>>,
    pub a2a_delegation: Option<Arc<dyn crate::a2a::A2aDelegationService>>,
    pub procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    pub ward_usage: Arc<gateway_services::WardUsage>,

    // --- Shared late-binding state ---
    /// ArcSwap so pre-captured contexts observe registries installed after
    /// construction (see the model-registry fallback incident).
    pub model_registry: Arc<arc_swap::ArcSwapOption<gateway_services::models::ModelRegistry>>,
    pub rate_limiters: Arc<RwLock<HashMap<String, Arc<agent_runtime::ProviderRateLimiter>>>>,
    pub integrations: SharedIntegrations,
    pub steering_registry: Arc<agent_runtime::SteeringRegistry>,
    pub agent_result_bus: Arc<AgentResultBus>,
    /// Per-ward serialization locks, shared across every delegation path.
    pub ward_locks: Arc<super::delegation_dispatcher::WardLocks>,
}

#[async_trait::async_trait]
impl super::session_invoker::ContinuationSpawner for ExecCtx {
    async fn spawn_continuation(
        &self,
        session_id: String,
        root_agent_id: String,
    ) -> Result<(), ExecutionError> {
        if let Err(error) = self.state_service.clear_continuation(&session_id) {
            tracing::warn!(%session_id, %error, "Failed to clear continuation flag");
        }
        let result =
            super::continuation_execution::invoke_continuation(self, &session_id, &root_agent_id)
                .await;
        if let Err(error) = result {
            // An unstarted continuation must not leave the session hanging:
            // publish the crash lifecycle so state and UI converge. The raw
            // error stays in tracing; the bus sees the constant safe message.
            tracing::error!(
                session_id = %session_id,
                root_agent_id = %root_agent_id,
                %error,
                "spawn_continuation failed; crashing session"
            );
            let execution_id = self
                .state_service
                .get_root_execution(&session_id)
                .ok()
                .flatten()
                .map(|execution| execution.id)
                .unwrap_or_default();
            const SAFE_CONTINUATION_ERROR: &str = "Unable to resume this session";
            crate::lifecycle::crash_execution(crate::lifecycle::CrashExecution {
                state_service: &self.state_service,
                log_service: &self.log_service,
                event_bus: &self.event_bus,
                execution_id: &execution_id,
                session_id: &session_id,
                agent_id: &root_agent_id,
                conversation_id: &session_id,
                error: SAFE_CONTINUATION_ERROR,
                crash_session: true,
            })
            .await;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl super::session_invoker::DelegationSpawner for ExecCtx {
    async fn spawn_delegation(
        &self,
        request: crate::delegation::DelegationRequest,
        permit: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<(), ExecutionError> {
        use crate::delegation::spawn::spawn_delegated_agent;

        // Bump per-ward usage telemetry before any locking; best-effort.
        let ward_name = request.child_agent_id.strip_prefix("ward:");
        if let Some(ward) = ward_name {
            if let Err(e) = self.ward_usage.bump_use(ward) {
                tracing::warn!(ward = %ward, error = %e, "ward_usage.bump_use failed");
            }
        }

        // Serialize ward-agent delegations per ward: ward-shared files are
        // written without filesystem locks; one ward-agent per ward at a time.
        let _ward_guard = match ward_name {
            Some(ward) => {
                Some(super::delegation_dispatcher::acquire_ward_lock(&self.ward_locks, ward).await)
            }
            None => None,
        };
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
            self.state_service.clone(),
            permit,
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
        .await
        .map(|_| ())
    }
}
