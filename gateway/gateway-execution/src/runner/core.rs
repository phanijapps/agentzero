//! # Execution Runner
//!
//! High-level API for agent execution and event streaming.
//!
//! The `ExecutionRunner` is the main entry point for invoking agents. It provides:
//! - Agent invocation with streaming events
//! - Execution control (stop, pause, resume, cancel)
//! - Agent delegation handling
//! - Session and execution lifecycle management

use agent_runtime::{ContextActorKind, ContextCapabilityCatalog, PreparedExecution};
use api_logs::LogService;
use execution_state::StateService;
use gateway_events::EventBus;
#[cfg(test)]
use gateway_events::GatewayEvent;
use gateway_services::{AgentService, McpService, ProviderService, SharedVaultPaths};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, Semaphore};
use zbot_runtime_sqlite::DatabaseManager;

/// Callback invoked after session creation but before any events are emitted.
/// Receives the session_id so the caller can set up subscriptions before events fire.
pub type OnSessionReady =
    Box<dyn FnOnce(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

// Import types from sibling modules
use crate::agent_pool::AgentResultBus;
pub use crate::config::ExecutionConfig;
use crate::delegation::{DelegationRegistry, DelegationRequest};
pub use crate::handle::ExecutionHandle;
use crate::invoke::{ExecutorBuilder, RuntimeActorKind};

// ============================================================================
// EXECUTION RUNNER
// ============================================================================

/// Execution runner that manages agent invocations.
///
/// The runner is responsible for:
/// - Creating and managing agent executors
/// - Processing delegation requests from running agents
/// - Tracking execution handles for control operations
/// - Broadcasting events to connected clients
pub struct ExecutionRunner {
    /// Event bus for broadcasting events
    pub(super) event_bus: Arc<EventBus>,
    /// Agent service for loading agent configs
    pub(super) agent_service: Arc<AgentService>,
    /// Provider service for loading provider configs
    pub(super) provider_service: Arc<ProviderService>,
    /// MCP service for loading MCP server configs
    pub(super) mcp_service: Arc<McpService>,
    /// Skill service for loading skill configs
    pub(super) skill_service: Arc<gateway_services::SkillService>,
    /// Vault paths for accessing configuration and data directories
    pub(super) paths: SharedVaultPaths,
    /// Live control owns the shared execution handles and persisted control transitions.
    pub(super) control: super::session_control::SessionControl,
    /// Message store (append-only conversation log).
    pub(super) messages: Arc<dyn zbot_conversation::MessageStore>,
    /// Narrow session metadata reads.
    pub(super) session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    /// Versioned agent-state checkpoints — written at each turn boundary.
    pub(super) checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    /// Channel for delegation requests
    pub(super) delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    /// Log service for execution tracing
    pub(super) log_service: Arc<LogService<DatabaseManager>>,
    /// Connector registry for response routing to external connectors
    pub(super) connector_registry: Option<Arc<gateway_connectors::ConnectorRegistry>>,
    /// Bridge registry for WebSocket worker connections
    pub(super) bridge_registry: Option<Arc<gateway_bridge::BridgeRegistry>>,
    /// Bridge outbox for reliable message delivery
    pub(super) bridge_outbox: Option<Arc<gateway_bridge::OutboxRepository>>,
    /// Trait-routed memory store.
    pub(super) memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    /// Session distiller for automatic fact extraction after sessions
    pub(super) distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    /// Handoff writer for session completion → KV summary.
    pub(super) handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
    /// Memory recall for automatic fact retrieval at session start
    pub(super) memory_recall: Option<Arc<crate::recall::MemoryRecall>>,
    pub(super) peer_messages: Option<Arc<crate::peer_messaging::DurablePeerMessageService>>,
    pub(super) a2a_delegation: Option<Arc<dyn crate::a2a::A2aDelegationService>>,
    /// Semaphore to limit concurrent delegation spawns (prevents resource exhaustion)
    pub(super) delegation_semaphore: Arc<Semaphore>,
    /// Embedding client for generating vector embeddings (semantic search in memory)
    pub(super) embedding_client: Option<Arc<dyn agent_runtime::llm::embedding::EmbeddingClient>>,
    /// Model capabilities registry for context window and capability lookups.
    ///
    /// Stored in an `ArcSwapOption` so the `RunnerContinuationInvoker`
    /// pre-captured by `ContinuationWatcher` (constructed before
    /// [`Self::set_model_registry`] is called during runtime wiring) can
    /// still observe the registry once it's installed. A plain
    /// `Option<Arc<ModelRegistry>>` would freeze as `None` in any
    /// pre-spawned task's captured clone — the original cause of the
    /// `context_window_tokens = 8192` fallback on the continuation path
    /// during executor construction.
    pub(super) model_registry:
        Arc<arc_swap::ArcSwapOption<gateway_services::models::ModelRegistry>>,
    /// Per-provider rate limiters — shared across all executors using the same provider.
    pub(super) rate_limiters: std::sync::Arc<
        std::sync::RwLock<
            std::collections::HashMap<String, std::sync::Arc<agent_runtime::ProviderRateLimiter>>,
        >,
    >,
    /// Current late-wired services, shared with bootstrap and captured invokers.
    pub(super) integrations: super::integrations::SharedIntegrations,
    pub(super) steering_registry: Arc<agent_runtime::SteeringRegistry>,
    pub(super) agent_result_bus: Arc<AgentResultBus>,
    /// Trait-routed procedure store for the `run_procedure` tool — wired by AppState.
    pub(super) procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    /// Per-ward usage telemetry — every `ward:<name>` delegation bumps it,
    /// the curator reads it. Wired by AppState via [`ExecutionRunnerConfig`].
    pub(super) ward_usage: Arc<gateway_services::WardUsage>,
    /// Pre-session setup delegate. Holds the dependency set needed by
    /// `invoke_with_callback`'s bootstrap phase, extracted here so
    /// `setup()` can be tested and read independently of the full runner.
    pub(super) bootstrap: super::invoke_bootstrap::InvokeBootstrap,
}

/// All inputs needed to construct an [`ExecutionRunner`].
///
/// Replaces the previous 18-positional-argument `with_connector_registry`
/// constructor. Using a struct literal at the call site means:
///
/// - Adding a new dependency is one line here + one line at every caller,
///   no positional reshuffling.
/// - Same-type `Option<Arc<...>>` fields (connector_registry vs bridge_registry
///   vs memory_store) can't be silently swapped — the field name is checked at
///   compile time.
/// - Callers that only want the minimum can lean on `Default::default()` for
///   the optional integrations.
pub struct ExecutionRunnerConfig {
    // --- Required services ---
    pub event_bus: Arc<EventBus>,
    pub agent_service: Arc<AgentService>,
    pub provider_service: Arc<ProviderService>,
    pub paths: SharedVaultPaths,
    pub mcp_service: Arc<McpService>,
    pub skill_service: Arc<gateway_services::SkillService>,
    pub log_service: Arc<LogService<DatabaseManager>>,
    pub state_service: Arc<StateService<DatabaseManager>>,
    /// Per-ward usage telemetry — feeds the curator. Required so every
    /// `ward:<name>` delegation can `bump_use` persistently.
    pub ward_usage: Arc<gateway_services::WardUsage>,
    /// New message store (T11 — writes route here via BatchWriter).
    pub messages: Arc<dyn zbot_conversation::MessageStore>,
    /// Narrow session metadata reads used while retiring the old repository.
    pub session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    /// Versioned checkpoints (T11 — written at each turn boundary).
    pub checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,

    // --- Optional integrations ---
    pub connector_registry: Option<Arc<gateway_connectors::ConnectorRegistry>>,
    /// Trait-routed memory store — wired.
    pub memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,
    pub distiller: Option<Arc<crate::distillation::SessionDistiller>>,
    pub handoff_writer: Option<Arc<crate::sleep::HandoffWriter>>,
    pub memory_recall: Option<Arc<crate::recall::MemoryRecall>>,
    pub peer_messages: Option<Arc<crate::peer_messaging::DurablePeerMessageService>>,
    pub a2a_delegation: Option<Arc<dyn crate::a2a::A2aDelegationService>>,
    pub bridge_registry: Option<Arc<gateway_bridge::BridgeRegistry>>,
    pub bridge_outbox: Option<Arc<gateway_bridge::OutboxRepository>>,
    pub embedding_client: Option<Arc<dyn agent_runtime::llm::embedding::EmbeddingClient>>,
    /// Trait-routed procedure store for the `run_procedure` tool.
    pub procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    /// Procedure recommendation tier thresholds (graduated promoted/advisory/tentative).
    /// Wired from `settings.memory.procedureRecommendation` by AppState.
    pub procedure_recommendation_cfg: gateway_memory::ProcedureRecommendationConfig,

    // --- Resource control ---
    pub max_parallel_agents: u32,
}

/// Wire the mid-session recall hook onto prepared inputs if the owning
/// runner has a [`MemoryRecall`] configured with `mid_session_recall.enabled`.
///
/// Same closure body is wired at two points — after a root executor is built
/// in `create_executor`, and after a continuation executor is built in
/// `invoke_continuation`. Extracted here so the ~55-line `set_recall_hook`
/// invocation lives in exactly one place; either call site that forgets it
/// must explicitly opt out rather than silently diverge.
pub(super) fn attach_mid_session_recall_hook(
    executor: &mut PreparedExecution,
    memory_recall: Option<&Arc<crate::recall::MemoryRecall>>,
    goals: Option<&Arc<dyn agent_tools::GoalAccess>>,
    agent_id: &str,
    session_id: &str,
    ward_id: Option<&str>,
    initial_recall_keys: std::collections::HashSet<String>,
) {
    let Some(recall) = memory_recall else {
        return;
    };
    let mid_cfg = &recall.config().mid_session_recall;
    if !mid_cfg.enabled {
        return;
    }

    let recall = Arc::clone(recall);
    let Some(authorization) = crate::invoke::unified_recall_adapter::recall_authorization_context(
        &recall, agent_id, "root", session_id, ward_id,
    ) else {
        tracing::debug!(
            agent_id,
            "Mid-session recall unavailable without provider scope"
        );
        return;
    };
    let goals = goals.cloned();
    let agent_id = agent_id.to_string();
    let ward = ward_id.map(String::from);
    let min_novelty = mid_cfg.min_novelty_score;
    let every_n = mid_cfg.every_n_turns as u32;

    executor.set_recall_hook(
        Box::new(
            move |query: &str, already_injected: &std::collections::HashSet<String>| {
                let recall = Arc::clone(&recall);
                let goals = goals.clone();
                let authorization = authorization.clone();
                let agent_id = agent_id.clone();
                let ward = ward.clone();
                let query = query.to_string();
                let already_injected = already_injected.clone();
                Box::pin(async move {
                    let mut response =
                        crate::invoke::unified_recall_adapter::automatic_unified_recall(
                            recall,
                            goals,
                            authorization,
                            query,
                            5,
                        )
                        .await
                        .map_err(|error| error.safe_message().to_string())?;
                    // Source-qualified generic IDs keep every unified source
                    // deduplicated without treating coincident IDs from two
                    // different source families as the same record.
                    retain_novel_unified_items(&mut response, &already_injected, min_novelty);
                    if response.results.is_empty() {
                        return Ok(agent_runtime::RecallHookResult {
                            system_message: String::new(),
                            fact_keys: Vec::new(),
                        });
                    }
                    let keys = response
                        .results
                        .iter()
                        .map(crate::recall::unified_item_dedup_key)
                        .collect();
                    let formatted = crate::recall::format_unified_recall_response_with_options(
                        &response,
                        crate::recall::ContextPacketBuildOptions::new(
                            format!("{agent_id}:mid-session-recall"),
                            agent_id,
                            ContextActorKind::Root,
                            900,
                        )
                        .with_ward_id(ward),
                    );
                    Ok(agent_runtime::RecallHookResult {
                        system_message: format_mid_session_recall_message(&formatted),
                        fact_keys: keys,
                    })
                })
            },
        ),
        every_n,
        initial_recall_keys,
    );
    tracing::debug!(every_n_turns = every_n, "Mid-session recall hook wired");
}

fn retain_novel_unified_items(
    response: &mut agent_tools::UnifiedRecallResponse,
    already_injected: &std::collections::HashSet<String>,
    min_novelty: f64,
) {
    response.results.retain(|item| {
        !already_injected.contains(&crate::recall::unified_item_dedup_key(item))
            && item.score >= min_novelty
    });
    response.count = response.results.len();
}

fn format_mid_session_recall_message(context: &str) -> String {
    format!(
        "[Memory Refresh] Relevant recalled context.\n{}\n{}",
        crate::recall::recall_untrusted_reference_notice(),
        context
    )
}

impl ExecutionRunner {
    /// Create a new execution runner from a [`ExecutionRunnerConfig`].
    ///
    /// Initializes the runner and spawns background tasks for processing
    /// delegation + continuation requests.
    pub fn with_config(config: ExecutionRunnerConfig) -> Self {
        let ExecutionRunnerConfig {
            event_bus,
            agent_service,
            provider_service,
            paths,
            mcp_service,
            skill_service,
            log_service,
            state_service,
            connector_registry,
            memory_store,
            distiller,
            handoff_writer,
            memory_recall,
            peer_messages,
            a2a_delegation,
            bridge_registry,
            bridge_outbox,
            embedding_client,
            procedure_store,
            max_parallel_agents,
            ward_usage,
            messages,
            session_meta,
            checkpoints,
            procedure_recommendation_cfg: _procedure_recommendation_cfg,
        } = config;

        // Create channel for delegation requests
        let (delegation_tx, delegation_rx) = mpsc::unbounded_channel::<DelegationRequest>();

        // Shared data structures — constructed once and Arc-cloned into both the
        // runner fields and the bootstrap.
        let handles: Arc<RwLock<HashMap<String, ExecutionHandle>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let delegation_registry = Arc::new(DelegationRegistry::new());
        let delegation_semaphore = Arc::new(Semaphore::new(max_parallel_agents as usize));
        let model_registry: Arc<arc_swap::ArcSwapOption<gateway_services::models::ModelRegistry>> =
            Arc::new(arc_swap::ArcSwapOption::from(None));
        let rate_limiters: std::sync::Arc<
            std::sync::RwLock<
                std::collections::HashMap<
                    String,
                    std::sync::Arc<agent_runtime::ProviderRateLimiter>,
                >,
            >,
        > = std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));
        let steering_registry = Arc::new(agent_runtime::SteeringRegistry::new());
        let agent_result_bus = Arc::new(AgentResultBus::new());
        let integrations = super::integrations::SharedIntegrations::default();

        let bootstrap = super::invoke_bootstrap::InvokeBootstrap {
            agent_service: agent_service.clone(),
            provider_service: provider_service.clone(),
            mcp_service: mcp_service.clone(),
            skill_service: skill_service.clone(),
            state_service: state_service.clone(),
            log_service: log_service.clone(),
            messages: messages.clone(),
            paths: paths.clone(),
            memory_store: memory_store.clone(),
            memory_recall: memory_recall.clone(),
            peer_messages: peer_messages.clone(),
            a2a_delegation: a2a_delegation.clone(),
            model_registry: model_registry.clone(),
            rate_limiters: rate_limiters.clone(),
            connector_registry: connector_registry.clone(),
            bridge_registry: bridge_registry.clone(),
            bridge_outbox: bridge_outbox.clone(),
            integrations: integrations.clone(),
            steering_registry: Some(steering_registry.clone()),
            agent_result_bus: Some(agent_result_bus.clone()),
            procedure_store: procedure_store.clone(),
            ward_usage: ward_usage.clone(),
            event_bus: event_bus.clone(),
            handles: handles.clone(),
        };

        let runner = Self {
            control: super::session_control::SessionControl {
                handles,
                delegation_registry,
                state_service,
            },
            event_bus,
            agent_service,
            provider_service,
            mcp_service,
            skill_service,
            paths,
            messages,
            session_meta,
            checkpoints,
            delegation_tx,
            log_service,
            connector_registry,
            bridge_registry,
            bridge_outbox,
            memory_store,
            distiller,
            handoff_writer,
            memory_recall,
            peer_messages,
            a2a_delegation,
            delegation_semaphore,
            embedding_client,
            model_registry,
            rate_limiters,
            integrations,
            steering_registry,
            agent_result_bus,
            procedure_store,
            ward_usage,
            bootstrap,
        };

        // Spawn delegation handler task — extracted into DelegationDispatcher.
        super::delegation_dispatcher::DelegationDispatcher {
            delegation_rx,
            delegation_semaphore: runner.delegation_semaphore.clone(),
            invoker: std::sync::Arc::new(runner.make_delegation_invoker()),
        }
        .spawn();

        // Spawn continuation watcher — extracted from the old inline
        // `spawn_continuation_handler` closure so the event-loop logic
        // is testable independently.
        super::continuation_watcher::ContinuationWatcher {
            event_bus: runner.event_bus.clone(),
            invoker: Arc::new(runner.make_continuation_invoker()),
        }
        .spawn();

        runner
    }

    /// Set the model capabilities registry.
    ///
    /// Takes `&self` (not `&mut self`) because the field is now an
    /// `Arc<ArcSwapOption<...>>` shared with the continuation handler
    /// task spawned during [`Self::new`]. The store is lock-free and
    /// becomes visible to subsequent `.load_full()` reads — which is
    /// what the continuation path does at fire time.
    pub fn set_model_registry(&self, registry: Arc<gateway_services::models::ModelRegistry>) {
        self.model_registry.store(Some(registry));
    }

    /// Build the peer handler with the same store, state, and live steering registry.
    pub fn peer_message_handler(&self) -> Option<Arc<dyn gateway_bus::WorkHandler>> {
        let service = self.peer_messages.as_ref()?;
        Some(Arc::new(crate::peer_messaging::PeerMessageHandler::new(
            service.store(),
            self.control.state_service.clone(),
            self.steering_registry.clone(),
            crate::peer_messaging::PEER_MESSAGE_TARGET,
        )))
    }

    pub fn steering_registry(&self) -> Arc<agent_runtime::SteeringRegistry> {
        self.steering_registry.clone()
    }

    /// Set the KG episode store used by post-distillation ward indexing.
    pub fn set_kg_episode_store(&mut self, store: Arc<dyn zbot_stores_traits::KgEpisodeStore>) {
        self.integrations.set_kg_episode_store(store);
    }

    /// Install the graph store for current and pre-captured execution paths.
    pub fn set_kg_store(&mut self, store: Arc<dyn zbot_stores::KnowledgeGraphStore>) {
        self.integrations.set_kg_store(store);
    }

    /// Install the ingestion adapter for all execution paths.
    pub fn set_ingestion_adapter(&mut self, adapter: Arc<dyn agent_tools::IngestionAccess>) {
        self.integrations.set_ingestion_adapter(adapter);
    }

    /// Install the goal adapter for all execution paths.
    pub fn set_goal_adapter(&mut self, adapter: Arc<dyn agent_tools::GoalAccess>) {
        self.integrations.set_goal_adapter(adapter);
    }

    /// Build a context capability catalog from the runner's live execution
    /// dependencies without starting an agent execution.
    pub fn context_capability_catalog(
        &self,
        actor_kind: RuntimeActorKind,
        tool_settings: agent_tools::ToolSettings,
        session_id: Option<String>,
        agent_id: Option<String>,
    ) -> ContextCapabilityCatalog {
        let integrations = self.integrations.snapshot();
        let mut builder = ExecutorBuilder::new(self.paths.vault_dir().clone(), tool_settings)
            .with_actor_kind(actor_kind)
            .with_state_service(self.control.state_service.clone())
            .with_message_store(self.messages.clone());

        if let Some(registry) = self.model_registry.load_full() {
            builder = builder.with_model_registry(registry);
        }
        if let Some(store) = &self.memory_store {
            builder = builder.with_fact_store(store.clone());
        }
        if let Some(provider) = self.connector_resource_provider() {
            builder = builder.with_connector_provider(provider);
        }
        if let Some(store) = integrations.kg_store {
            builder = builder.with_kg_store(store);
        }
        if let Some(adapter) = integrations.ingestion_adapter {
            builder = builder.with_ingestion_adapter(adapter);
        }
        if let Some(adapter) = integrations.goal_adapter {
            builder = builder.with_goal_adapter(adapter);
        }
        if let Some(store) = &self.procedure_store {
            builder = builder.with_procedure_store(store.clone());
        }
        if let Some(recall) = &self.memory_recall {
            builder = builder.with_memory_recall(recall.clone());
        }

        let observer = Arc::new(crate::invoke::ward_usage_adapter::WardUsageAdapter::new(
            self.ward_usage.clone(),
        ));
        builder = builder
            .with_ward_usage(observer)
            .with_ward_usage_service(self.ward_usage.clone())
            .with_steering_registry(self.steering_registry.clone())
            .with_agent_result_bus(self.agent_result_bus.clone());

        builder.build_context_capability_catalog(session_id, agent_id)
    }

    fn connector_resource_provider(
        &self,
    ) -> Option<Arc<dyn agent_primitives::ConnectorResourceProvider>> {
        let http_provider: Option<Arc<dyn agent_primitives::ConnectorResourceProvider>> =
            self.connector_registry.as_ref().map(|registry| {
                Arc::new(crate::resource_provider::GatewayResourceProvider::new(
                    registry.clone(),
                )) as Arc<dyn agent_primitives::ConnectorResourceProvider>
            });
        let bridge_provider: Option<Arc<dyn agent_primitives::ConnectorResourceProvider>> = self
            .bridge_registry
            .as_ref()
            .zip(self.bridge_outbox.as_ref())
            .map(|(registry, outbox)| {
                Arc::new(gateway_bridge::BridgeResourceProvider::new(
                    registry.clone(),
                    outbox.clone(),
                )) as Arc<dyn agent_primitives::ConnectorResourceProvider>
            });

        if http_provider.is_some() || bridge_provider.is_some() {
            Some(Arc::new(
                crate::composite_provider::CompositeResourceProvider::new(
                    http_provider,
                    bridge_provider,
                ),
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod delegation_flow_tests;
#[cfg(test)]
mod mid_session_recall_tests;
#[cfg(test)]
mod model_registry_late_binding_tests;
#[cfg(test)]
mod peer_root_lifecycle_tests;
#[cfg(test)]
mod setup_failure_cleanup_tests;
#[cfg(test)]
mod test_support;
