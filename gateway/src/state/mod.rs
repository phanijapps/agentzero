//! # Application State
//!
//! Shared state for the gateway application.

mod capability_catalog;
pub(crate) mod persistence_factory;
mod seeded_defaults;

use crate::connectors::{ConnectorRegistry, ConnectorService};
use crate::cron::CronScheduler;
use crate::events::EventBus;
use crate::execution::{DelegationRegistry, MemoryRecall, SessionArchiver};
use crate::hooks::HookRegistry;
use crate::services::{
    AgentService, McpService, ModelRegistry, ProviderService, RuntimeService, SettingsService,
    SharedVaultPaths, SkillService, VaultPaths,
};
use agent_runtime::llm::EmbeddingClient;
use api_logs::LogService;
use execution_state::{SqliteWorkStore, StateService, WorkStore};
use gateway_services::EmbeddingService;
use std::path::PathBuf;
use std::sync::Arc;
use zbot_engram_adapter::GovernanceCapabilityHealth;
use zbot_runtime_sqlite::{DatabaseManager, DistillationRepository};

/// Shared application state for the gateway.
#[derive(Clone)]
pub struct AppState {
    /// Agent service for managing agent configurations.
    pub agents: Arc<AgentService>,

    /// Skill service for managing skill configurations.
    pub skills: Arc<SkillService>,

    /// Provider service for managing LLM providers.
    pub provider_service: Arc<ProviderService>,

    /// MCP service for managing MCP server configurations.
    pub mcp_service: Arc<McpService>,

    /// Runtime service for agent execution.
    pub runtime: Arc<RuntimeService>,

    /// Event bus for broadcasting events.
    pub event_bus: Arc<EventBus>,

    /// Hook registry for managing inbound triggers.
    pub hook_registry: Option<Arc<HookRegistry>>,

    /// Delegation registry for tracking agent delegations.
    pub delegation_registry: Arc<DelegationRegistry>,

    /// Message store (append-only conversation log).
    pub messages: Arc<dyn zbot_conversation::MessageStore>,
    /// Narrow session metadata reads used while retiring the old repository.
    pub session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    /// Versioned agent-state checkpoints — `session_state` reads here (T12).
    pub checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    /// Durable operational decision threads. Semantic memory remains in Engram.
    pub autonomy: Arc<dyn zbot_conversation::AutonomyStore>,
    /// Slim (payload-free) `execution_logs` — the live `/api/logs` UI source.
    pub slim_logs: Arc<dyn zbot_trace::SlimLogStore>,
    /// Cross-session trace analytics over `traces/*.jsonl.gz` (DuckDB).
    pub trace_analytics: Arc<zbot_trace::TraceAnalytics>,

    /// Settings service for application configuration.
    pub settings: Arc<SettingsService>,

    /// Log service for execution tracing.
    pub log_service: Arc<LogService<DatabaseManager>>,

    /// State service for execution state management.
    pub state_service: Arc<StateService<DatabaseManager>>,

    /// Durable executable-work store backed by the conversation database.
    pub durable_work_store: Arc<dyn WorkStore>,

    /// Shared in-process wake path for durable executable work.
    pub durable_work_transport: Arc<gateway_bus::LocalWorkTransport>,

    /// Connector registry for external bridge management.
    pub connector_registry: Arc<ConnectorRegistry>,

    /// Bridge registry for WebSocket worker connections.
    pub bridge_registry: Arc<gateway_bridge::BridgeRegistry>,

    /// Bridge outbox for reliable message delivery to workers.
    pub bridge_outbox: Arc<gateway_bridge::OutboxRepository>,

    /// Gateway bus for bridge inbound message routing (set during server start).
    pub bridge_bus: Option<Arc<dyn gateway_bus::GatewayBus>>,

    /// Trait-routed memory-fact store. The single read/write surface for
    /// memory facts.
    pub memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>>,

    /// Backend-neutral active goals used for intent boost and the goal tool.
    pub goal_store: Option<Arc<dyn zbot_stores_traits::GoalStore>>,

    /// Distillation repository for tracking distillation run outcomes.
    pub distillation_repo: Option<Arc<DistillationRepository>>,

    /// Session distiller for triggering on-demand distillation (e.g., backfill).
    pub distiller: Option<Arc<distillation::SessionDistiller>>,

    /// Backend-neutral session episode store.
    pub episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>>,

    /// Trait-routed wiki store (Phase D3). The handler-side migrations
    /// route through this; legacy callers still build a
    /// `WardWikiRepository` directly. `None` in minimal AppStates.
    pub wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>>,

    /// Trait-routed procedure store (Phase D4).
    pub procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,

    /// Backend-neutral kg-ingestion-episode store.
    pub kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>>,

    /// Trait-based knowledge-graph store.
    pub kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>>,

    /// Additive path-free governance health for Observatory/read-model routes.
    pub governance_health: Option<GovernanceCapabilityHealth>,

    /// Streaming ingestion queue (Phase 2) — None when graph is unavailable.
    pub ingestion_queue: Option<Arc<gateway_execution::ingest::IngestionQueue>>,

    /// Per-source + global backpressure gate for `/api/graph/ingest`.
    pub ingestion_backpressure: Option<Arc<gateway_execution::ingest::Backpressure>>,

    /// Cron scheduler for scheduled agent triggers.
    /// Optional because it requires async initialization with GatewayBus.
    pub cron_scheduler: Option<Arc<CronScheduler>>,

    /// Plugin manager for STDIO plugin lifecycle.
    pub plugin_manager: Arc<gateway_bridge::PluginManager>,

    /// Session archiver for offloading old transcripts to compressed files.
    pub session_archiver: Option<Arc<SessionArchiver>>,

    /// Sleep-time worker — triggers graph compaction/consolidation cycles.
    /// Set by server.start() in Phase 4 Task 10; `None` until then.
    pub sleep_time_worker: Option<Arc<gateway_memory::sleep::SleepTimeWorker>>,

    /// Trait-routed compaction audit store. Wired in both
    /// SQLite and SurrealDB modes — the maintenance worker writes
    /// merge/prune/synthesis events here for Observatory display.
    /// Backend-agnostic: the trait has default no-op impls so any
    /// backend that doesn't care can inherit them.
    pub compaction_store: Option<Arc<dyn zbot_stores_traits::CompactionStore>>,

    /// Trait-routed belief store (Belief Network Phase B-5 HTTP surface).
    /// `Some(...)` only when `execution.memory.beliefNetwork.enabled = true`
    /// AND the knowledge DB is wired. The HTTP handlers in
    /// `http::beliefs` and `http::belief_network` use this for 503-vs-200
    /// disambiguation: a `None` here means the Belief Network is disabled,
    /// not that the data is missing.
    pub belief_store: Option<Arc<dyn zbot_stores_traits::BeliefStore>>,

    /// Trait-routed belief-contradiction store (Belief Network Phase B-5
    /// HTTP surface). Same opt-in gating as `belief_store`.
    pub belief_contradiction_store: Option<Arc<dyn zbot_stores_traits::BeliefContradictionStore>>,

    /// In-memory recorder of recent Belief Network worker stats (Phase
    /// B-6). Always wired when the sleep-time worker is wired so the
    /// HTTP layer can render the Observatory belief panel even when the
    /// network itself is disabled (empty history + `enabled: false`).
    pub belief_network_activity: Option<Arc<gateway_memory::RecentBeliefNetworkActivity>>,

    /// Fallback-only model metadata registry.
    pub model_registry: Arc<ModelRegistry>,

    /// Embedding service — owns live EmbeddingClient, supports backend swap.
    pub embedding_service: Arc<EmbeddingService>,

    /// Vault paths for accessing configuration and data directories.
    pub paths: SharedVaultPaths,

    /// Vault root path. Prefer `paths` for child locations.
    pub vault_dir: PathBuf,

    /// LAN service advertiser. NoopAdvertiser when discovery is disabled.
    pub advertiser: std::sync::Arc<dyn discovery::Advertiser>,

    /// Active mDNS advertise handle. None until `start()` runs and only
    /// populated when `network.exposeToLan = true`.
    pub advertise_handle: std::sync::Arc<std::sync::Mutex<Option<discovery::AdvertiseHandle>>>,
}

type ConversationStoreBundle = (
    Arc<dyn zbot_conversation::MessageStore>,
    Arc<dyn zbot_conversation::SessionMetaStore>,
    Arc<dyn zbot_conversation::CheckpointStore>,
    Arc<dyn zbot_conversation::AutonomyStore>,
    Arc<dyn zbot_trace::SlimLogStore>,
    Arc<zbot_trace::TraceAnalytics>,
);

/// Construct the new conversation/trace stores sharing **one** r2d2 pool, with
/// both crates' schemas initialized on it (`messages`+`checkpoints` via
/// `open_conversation_pool`; `execution_logs` here). The old `conversations`
/// field stays in place until the T16 cutover delete.
fn build_conversation_stores(paths: &SharedVaultPaths) -> anyhow::Result<ConversationStoreBundle> {
    let pool = zbot_conversation::open_conversation_pool(&paths.conversations_db())?;
    {
        let conn = pool.get()?;
        zbot_trace::schema::initialize(&conn)?;
    }
    paths.ensure_optional_dir(&paths.traces_dir())?;
    Ok((
        Arc::new(zbot_conversation::SqliteMessageStore::new(pool.clone())),
        Arc::new(zbot_conversation::SqliteSessionMetaStore::new(pool.clone())),
        Arc::new(zbot_conversation::SqliteCheckpointStore::new(pool.clone())),
        Arc::new(zbot_conversation::SqliteAutonomyStore::new(pool.clone())),
        Arc::new(zbot_trace::SqliteSlimLogStore::new(pool)),
        Arc::new(zbot_trace::TraceAnalytics::open(&paths.traces_dir())?),
    ))
}

impl AppState {
    /// Create a new application state.
    ///
    /// This creates a fully initialized state with execution runner and SQLite database.
    pub fn new(vault_dir: PathBuf) -> Self {
        Self::new_with_a2a(vault_dir, false)
    }

    pub fn new_with_a2a(vault_dir: PathBuf, a2a_enabled: bool) -> Self {
        // Create centralized vault paths
        let paths = Arc::new(VaultPaths::new(vault_dir.clone()));

        // Ensure required directories exist
        if let Err(e) = paths.ensure_dirs_exist() {
            tracing::warn!("Failed to create vault directories: {}", e);
        }
        if let Err(e) = gateway_services::seed_default_ward_archetypes(&paths) {
            tracing::warn!("Failed to seed editable ward archetypes: {}", e);
        }
        if let Err(e) = paths.migrate_legacy_layout() {
            tracing::warn!("Failed to migrate legacy vault layout: {}", e);
        }

        let agents_dir = paths.agents_dir();
        let skills_roots = paths.skills_dirs();
        let event_bus = Arc::new(EventBus::new());
        let agents = Arc::new(AgentService::new(agents_dir));
        // Load skills from the vault first, then $HOME/.agents/skills.
        // Vault wins when both roots provide a skill with the same name.
        let skills = Arc::new(SkillService::with_roots(skills_roots));
        let provider_service = Arc::new(ProviderService::new(paths.clone()));
        let mcp_service = Arc::new(McpService::new(paths.clone()));
        let settings = Arc::new(SettingsService::new(paths.clone()));
        if let Err(code) = crate::http::commissioning::activate_pending_memory_profile_on_boot(
            paths.as_ref(),
            settings.as_ref(),
        ) {
            tracing::warn!(
                event = "commissioning_memory_profile_activation_deferred",
                code,
                "Pending Full memory profile was not finalized"
            );
        }
        let memory_provider_settings = settings
            .get_execution_settings()
            .map(|s| s.memory.provider.clone())
            .unwrap_or_default();

        // Factory for sleep-time memory LLM clients — built once, shared
        // across every sleep-time component that needs an LLM call.
        let memory_llm_factory: Arc<dyn gateway_memory::MemoryLlmFactory> = Arc::new(
            crate::memory_llm_factory::ProviderServiceLlmFactory::new(provider_service.clone()),
        );

        // Initialize fallback-only model metadata registry.
        let model_registry = Arc::new(ModelRegistry::load());

        // Initialize SQLite database for conversation persistence
        let db_manager = Arc::new(
            DatabaseManager::new(paths.clone())
                .expect("Failed to initialize conversation database"),
        );

        // Semantic memory/knowledge now lives behind Engram. The only zbot-owned
        // runtime SQLite DB opened here is conversations.db via DatabaseManager.
        tracing::info!("Engram memory provider selected; skipping SQLite knowledge DB init");

        // Create log service for execution tracing
        let log_service = Arc::new(LogService::new(db_manager.clone()));

        // Create state service for execution state management
        let state_service = Arc::new(StateService::new(db_manager.clone()));
        let durable_work_store: Arc<dyn WorkStore> =
            Arc::new(SqliteWorkStore::new(db_manager.clone()));
        let durable_work_transport = Arc::new(gateway_bus::LocalWorkTransport::new());
        state_service.set_surface_persistence_enabled(
            settings
                .get_presentation_settings()
                .map(|value| value.persist_surfaces)
                .unwrap_or(false),
        );

        // Create connector registry
        let connector_service = ConnectorService::new(paths.clone());
        let connector_registry = Arc::new(ConnectorRegistry::new(connector_service));

        // Create bridge registry and outbox for WebSocket workers
        let bridge_registry = Arc::new(gateway_bridge::BridgeRegistry::new());
        let bridge_outbox = Arc::new(gateway_bridge::OutboxRepository::new(db_manager.clone()));

        // Phase E6c: distillation_run rows live on the conversation DB
        // (DatabaseManager), not knowledge.db. Wire unconditionally —
        // both backends have the conversation DB. This makes
        // /api/distillation/status report real numbers
        // too, and the distiller's run-tracking (insert/retry/success)
        // actually persists.
        let distillation_repo: Option<Arc<DistillationRepository>> =
            Some(Arc::new(DistillationRepository::new(db_manager.clone())));

        // EmbeddingService — owns the live EmbeddingClient and supports
        // hot-swap between internal (fastembed) and Ollama backends.
        // Phase 1 of embedding-backend-selection: boot succeeds even if the
        // configured Ollama endpoint is unreachable; consumers continue
        // holding their Arc<dyn EmbeddingClient> cloned from service.client().
        let embedding_service = match EmbeddingService::from_config(paths.clone()) {
            Ok(svc) => Arc::new(svc),
            Err(e) => {
                tracing::warn!(
                    "EmbeddingService init failed ({e}); falling back to internal/384d default"
                );
                Arc::new(
                    EmbeddingService::with_config(paths.clone(), Default::default())
                        .expect("default EmbeddingService must build"),
                )
            }
        };
        tracing::info!("Engram memory provider selected; skipping SQLite embedding reindex");
        // Hand downstream (distillation, recall, memory_fact_store, etc.) a
        // LiveEmbeddingClient wrapper so they follow ArcSwap backend changes
        // instead of caching the boot-time client (which would still be the
        // Noop / Unconfigured client after the user later picks Ollama).
        let embedding_client: Option<Arc<dyn EmbeddingClient>> = Some(Arc::new(
            gateway_services::LiveEmbeddingClient::new(embedding_service.clone()),
        ));
        tracing::info!(
            "Embedding client ready (lazy, {}d)",
            embedding_service.dimensions()
        );

        let engram_store_bundle = Some(
            persistence_factory::build_engram_store_bundle(
                paths.as_ref(),
                &memory_provider_settings,
                embedding_client.clone(),
            )
            .expect("Failed to initialize selected Engram memory provider"),
        );
        tracing::info!("Engram memory provider initialized for trait-routed stores");

        // Load recall configuration (compiled defaults merged with optional user overrides)
        let recall_config = Arc::new(gateway_services::RecallConfig::load_from_file(
            &paths.recall_config(),
        ));

        // Create session archiver for offloading old transcripts to compressed files
        let archive_path = paths
            .data_dir()
            .join(&recall_config.session_offload.archive_path);
        let session_archiver = Arc::new(SessionArchiver::new(db_manager.clone(), archive_path));

        // Build the trait-routed memory_store eagerly (before MemoryRecall +
        // distillation::SessionDistiller construction, so they can be wired with it).
        let early_memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.memory_store.clone());

        // Create memory recall. Builds whenever the Engram memory store is
        // wired. Graph enrichment routes through the trait store; the old
        // concrete GraphService path is no longer opened here.
        let mut memory_recall_inner: Option<MemoryRecall> = if early_memory_store.is_some() {
            Some(MemoryRecall::new(
                embedding_client.clone(),
                recall_config.clone(),
            ))
        } else {
            None
        };
        if let Some(recall) = memory_recall_inner.as_mut() {
            let store_opt: Option<Arc<dyn zbot_stores_traits::EpisodeStore>> = engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.episode_store.clone());
            if let Some(store) = store_opt {
                recall.set_episode_store(store);
            }
        }

        // Wire trait-routed wiki_store (Phase E6c).
        if let Some(recall) = memory_recall_inner.as_mut() {
            let store_opt: Option<Arc<dyn zbot_stores_traits::WikiStore>> = engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.wiki_store.clone());
            if let Some(store) = store_opt {
                recall.set_wiki_store(store);
            }
        }
        // Trait-routed kg ingestion store.
        let kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.kg_episode_store.clone());

        // Trait-routed wiki store.
        let wiki_store_for_state: Option<Arc<dyn zbot_stores_traits::WikiStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.wiki_store.clone());

        let procedure_store_for_state: Option<Arc<dyn zbot_stores_traits::ProcedureStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.procedure_store.clone());
        // Wire the trait-routed procedure_store on MemoryRecall so
        // procedure recall runs (Phase E6c).
        if let (Some(recall), Some(ps)) = (
            memory_recall_inner.as_mut(),
            procedure_store_for_state.as_ref(),
        ) {
            recall.set_procedure_store(ps.clone());
        }

        // Trait-routed episode store for downstream consumers (distiller +
        // sleep worker + AppState). Built once here so the sleep worker
        // construction below doesn't have to re-derive backend-specific handles.
        let episode_store_for_state: Option<Arc<dyn zbot_stores_traits::EpisodeStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.episode_store.clone());

        if let (Some(recall), Some(mem)) =
            (memory_recall_inner.as_mut(), early_memory_store.as_ref())
        {
            recall.set_memory_store(mem.clone());
        }

        if let Some(recall) = memory_recall_inner.as_mut() {
            let taxonomy_expander: Option<Arc<dyn zbot_stores_traits::RecallTaxonomyExpander>> =
                engram_store_bundle
                    .as_ref()
                    .and_then(|bundle| bundle.taxonomy_expander.clone());
            // A configured Engram provider proves the tenant/workspace
            // boundary. The optional expander independently determines
            // whether a taxonomy source is configured; this lets unified
            // recall report `not_configured` instead of treating an absent
            // optional source as a scope failure.
            let taxonomy_scope_proven = engram_store_bundle.is_some();
            if let Some(taxonomy_expander) = taxonomy_expander {
                recall.set_taxonomy_expander(taxonomy_expander);
            }
            // This comes from the exact provider configuration used to open
            // the stores above, not from executor or model request state.
            // A non-workspace ward mapping intentionally leaves runtime
            // workspace unset; the adapter then proves tenant scope only.
            recall.set_provider_scope(gateway_memory::RecallProviderScope::new(
                memory_provider_settings.tenant.clone(),
                memory_provider_settings.ward_scope_target
                    == gateway_memory::MemoryScopeTarget::Workspace,
                taxonomy_scope_proven,
            ));
            if let Ok(settings) = gateway_services::SettingsService::new(paths.clone()).load() {
                let limits = settings.execution.memory.provider.governance.skos_expansion;
                recall.set_taxonomy_expansion_limits(gateway_memory::RecallSkosExpansionLimits {
                    max_depth: limits.max_depth,
                    max_fan_out: limits.max_fan_out,
                    max_candidates: limits.max_candidates,
                });
            }
        }

        // Build the trait-routed kg_store early enough to wire it on
        // MemoryRecall before that struct is moved into Arc::new below.
        let kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.kg_store.clone());
        if let (Some(recall), Some(ks)) = (memory_recall_inner.as_mut(), kg_store.as_ref()) {
            recall.set_kg_store(ks.clone());
        }

        // Phase B-4: wire BeliefStore into MemoryRecall, gated on
        // `execution.memory.beliefNetwork.enabled`. Reads settings
        // eagerly here so the store is attached before MemoryRecall is
        // sealed in `Arc::new` below. When the flag is off (default)
        // OR the Engram belief store is missing, no store is wired and recall
        // stays byte-for-byte identical to pre-B-4 behavior.
        let belief_network_enabled_for_recall =
            gateway_services::SettingsService::new(paths.clone())
                .load()
                .map(|s| s.execution.memory.belief_network.enabled)
                .unwrap_or(false);
        if belief_network_enabled_for_recall {
            let belief_store_for_recall: Option<Arc<dyn zbot_stores_traits::BeliefStore>> =
                engram_store_bundle
                    .as_ref()
                    .map(|bundle| bundle.belief_store.clone());
            if let Some(belief_store_for_recall) = belief_store_for_recall {
                if let Some(recall) = memory_recall_inner.as_mut() {
                    recall.set_belief_store(belief_store_for_recall);
                }
                tracing::info!("Belief Network recall: enabled (B-4 — beliefs in recall_unified)");
            } else {
                tracing::info!(
                    "Belief Network recall: enabled in settings but knowledge DB unavailable; skipping"
                );
            }
        } else {
            tracing::debug!("Belief Network recall: disabled (default)");
        }

        // MMR diversity reranking (default-on; disable via `memory.mmr.enabled` in
        // settings.json). Default-enabled: diversity reranking is part of
        // the production recall pipeline (P2). The config block is attached
        // unconditionally so the runtime can read current values; only
        // `enabled = true` triggers the rerank step inside `recall_unified`.
        let mmr_cfg: gateway_memory::MmrConfig =
            gateway_services::SettingsService::new(paths.clone())
                .load()
                .map(|s| s.execution.memory.mmr.clone())
                .unwrap_or_default();
        if let Some(recall) = memory_recall_inner.as_mut() {
            recall.set_mmr_config(mmr_cfg.clone());
        }
        if mmr_cfg.enabled {
            tracing::info!(
                "Memory MMR rerank: enabled (lambda={}, candidate_pool={})",
                mmr_cfg.lambda,
                mmr_cfg.candidate_pool,
            );
        } else {
            tracing::debug!("Memory MMR rerank: disabled (default)");
        }

        // Observatory v2 Phase 3 — wire the EventBus so recall_unified
        // can emit RecallTrace telemetry for the live canvas overlay.
        if let Some(recall) = memory_recall_inner.as_mut() {
            recall.set_event_bus(event_bus.clone());
        }

        let memory_recall: Option<Arc<MemoryRecall>> = memory_recall_inner.map(Arc::new);

        // Clone embedding client before it's moved into distiller — the runner
        // also needs it so the memory fact store can generate embeddings.
        let runner_embedding_client = embedding_client.clone();

        // Build the conversation stores before the distiller/runtime so both
        // use the same MessageStore/SessionMetaStore/CheckpointStore handles.
        let (messages, session_meta, checkpoints, autonomy, slim_logs, trace_analytics) =
            build_conversation_stores(&paths)
                .expect("Failed to initialize conversation/trace stores");

        // kg_store was built earlier (before memory_recall_inner moved
        // into Arc::new) so it could be wired on MemoryRecall. Both the
        // distiller and AppState fields below reuse the same Engram-backed
        // trait object.
        let memory_store = early_memory_store;

        // distillation::SessionDistiller writes semantic artifacts through Engram-backed
        // trait stores. Conversation-linked run tracking still uses
        // conversations.db through DistillationRepository.
        let distiller: Option<Arc<distillation::SessionDistiller>> = if memory_store.is_some() {
            let mut distiller_inner = distillation::SessionDistiller::new(
                provider_service.clone(),
                embedding_client.clone(),
                messages.clone(),
                session_meta.clone(),
                paths.clone(),
                Some(settings.clone()),
            );
            if let Some(mem) = memory_store.as_ref() {
                distiller_inner.set_memory_store(mem.clone());
            }
            if let Some(kgs) = kg_store.as_ref() {
                distiller_inner.set_kg_store(kgs.clone());
            }
            // Phase E6a/E6b: episode/wiki/procedure trait stores reuse the
            // same Arc<dyn ...> values we built above for the AppState
            // fields (`episode_store`, `wiki_store_for_state`,
            // `procedure_store_for_state`) so the distiller and the HTTP
            // handlers see the same backing store.
            if let Some(es) = episode_store_for_state.as_ref() {
                distiller_inner.set_episode_store(es.clone());
            }
            if let Some(ws) = wiki_store_for_state.as_ref() {
                distiller_inner.set_wiki_store(ws.clone());
            }
            if let Some(ps) = procedure_store_for_state.as_ref() {
                distiller_inner.set_procedure_store(ps.clone());
            }
            // Phase E6c: trait-routed distillation store. Wraps the
            // SQLite DistillationRepository for run-tracking writes.
            if let Some(dr) = distillation_repo.as_ref() {
                let store: Arc<dyn zbot_stores_traits::DistillationStore> = Arc::new(
                    zbot_runtime_sqlite::GatewayDistillationStore::new(dr.clone()),
                );
                distiller_inner.set_distillation_store(store);
            }
            Some(Arc::new(distiller_inner))
        } else {
            None
        };

        // Keep a handle for on-demand distillation (backfill, trigger).
        // None when the distiller wasn't constructed.
        let distiller_ref: Option<Arc<distillation::SessionDistiller>> = distiller.clone();
        let max_parallel_agents = settings
            .get_execution_settings()
            .map(|s| s.max_parallel_agents)
            .unwrap_or(2);
        tracing::info!(max_parallel_agents, "Execution settings loaded");
        let peer_messages = Arc::new(
            gateway_execution::peer_messaging::DurablePeerMessageService::new(
                durable_work_store.clone(),
                durable_work_transport.clone(),
                state_service.clone(),
                gateway_execution::peer_messaging::PEER_MESSAGE_TARGET,
            ),
        );
        let a2a_delegation = a2a_enabled.then(|| {
            Arc::new(crate::tasks::a2a::GatewayA2aDelegationService::new(
                paths.vault_dir(),
                durable_work_store.clone(),
                durable_work_transport.clone(),
                state_service.clone(),
            )) as Arc<dyn gateway_execution::a2a::A2aDelegationService>
        });

        // Create streaming ingestion queue + backpressure BEFORE the runtime so the
        // runner can be wired with an IngestionAdapter.
        //
        // Trait-routed: queue + backpressure consume
        // Arc<dyn KgEpisodeStore> + Arc<dyn KnowledgeGraphStore>.
        let (ingestion_queue, ingestion_backpressure) =
            match (kg_episode_store.as_ref(), kg_store.as_ref()) {
                (Some(eps), Some(kgs)) => {
                    let extractor =
                        Arc::new(gateway_execution::ingest::extractor::LlmExtractor::new(
                            provider_service.clone(),
                            "root".to_string(),
                        ));
                    let queue = Arc::new(gateway_execution::ingest::IngestionQueue::start(
                        2,
                        eps.clone(),
                        kgs.clone(),
                        extractor,
                    ));
                    let bp = Arc::new(gateway_execution::ingest::Backpressure::new(
                        gateway_execution::ingest::BackpressureConfig::default(),
                        eps.clone(),
                    ));
                    (
                        Some(queue) as Option<Arc<gateway_execution::ingest::IngestionQueue>>,
                        Some(bp) as Option<Arc<gateway_execution::ingest::Backpressure>>,
                    )
                }
                _ => (None, None),
            };

        // Build agent-tool adapters so runner can register `ingest` + `goal` tools.
        // Phase B2: also trait-routed. The IngestionAdapter is migrated
        // alongside the queue so subagent ingestion works.
        let ingestion_adapter: Option<Arc<dyn agent_tools::IngestionAccess>> = match (
            ingestion_queue.as_ref(),
            kg_store.as_ref(),
            kg_episode_store.as_ref(),
        ) {
            (Some(q), Some(kgs), Some(eps)) => Some(Arc::new(
                gateway_execution::invoke::ingest_adapter::IngestionAdapter::new(
                    q.clone(),
                    eps.clone(),
                    kgs.clone(),
                ),
            )
                as Arc<dyn agent_tools::IngestionAccess>),
            _ => None,
        };
        // Goal adapter is trait-routed through the Engram adapter sidecar.
        let goal_store_for_adapter: Option<Arc<dyn zbot_stores_traits::GoalStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.goal_store.clone());
        let goal_adapter: Option<Arc<dyn agent_tools::GoalAccess>> =
            goal_store_for_adapter.map(|store| {
                Arc::new(gateway_execution::invoke::goal_adapter::GoalAdapter::new(
                    store,
                )) as Arc<dyn agent_tools::GoalAccess>
            });

        // Create runtime with execution runner and connector registry
        // Belief Network handles, gated on the feature flag — the tool
        // surface (`belief` tool), HTTP surface, and AppState all share
        // this gating so each returns clean 503/"not configured" errors
        // when the feature is off.
        let belief_network_cfg = settings
            .get_execution_settings()
            .map(|s| s.memory.belief_network.clone())
            .unwrap_or_default();
        let belief_store_raw: Option<Arc<dyn zbot_stores_traits::BeliefStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.belief_store.clone());
        let belief_contradiction_store_raw: Option<
            Arc<dyn zbot_stores_traits::BeliefContradictionStore>,
        > = engram_store_bundle
            .as_ref()
            .map(|bundle| bundle.belief_contradiction_store.clone());
        let belief_store_for_http: Option<Arc<dyn zbot_stores_traits::BeliefStore>> =
            if belief_network_cfg.enabled {
                belief_store_raw.clone()
            } else {
                None
            };
        let belief_contradiction_store_for_http: Option<
            Arc<dyn zbot_stores_traits::BeliefContradictionStore>,
        > = if belief_network_cfg.enabled {
            belief_contradiction_store_raw.clone()
        } else {
            None
        };

        let runtime = Arc::new(RuntimeService::with_runner_and_connectors(
            event_bus.clone(),
            agents.clone(),
            provider_service.clone(),
            paths.clone(),
            messages.clone(),
            session_meta.clone(),
            checkpoints.clone(),
            mcp_service.clone(),
            skills.clone(),
            log_service.clone(),
            state_service.clone(),
            Some(peer_messages),
            a2a_delegation,
            Some(connector_registry.clone()),
            memory_store.clone(),
            distiller,
            memory_recall,
            Some(bridge_registry.clone()),
            Some(bridge_outbox.clone()),
            runner_embedding_client,
            max_parallel_agents,
            kg_store.clone(),
            None,
            ingestion_adapter,
            goal_adapter,
            procedure_store_for_state.clone(),
            belief_store_for_http.clone(),
            belief_contradiction_store_for_http.clone(),
            settings
                .load()
                .map(|s| s.execution.memory.procedure_recommendation.clone())
                .unwrap_or_default(),
            memory_llm_factory.clone(),
        ));

        // Phase 4: CompactionRepository + SleepTimeWorker (background maintenance).
        // The concrete SQLite compaction repository is retired from runtime
        // composition; Engram-backed compaction audit storage is wired below.

        // Phase D1: trait-routed compaction audit store. Wired in BOTH
        // backends so the maintenance worker can record merges/prunes
        // regardless of backend. Surreal uses its own
        // `kg_compaction_run` table; SQLite delegates to the existing
        // `CompactionRepository`. Default no-op impls cover edge cases.
        let compaction_store: Option<Arc<dyn zbot_stores_traits::CompactionStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.compaction_store.clone());

        // The legacy SQLite KG metadata backfill is retired from runtime
        // composition. Engram-backed stores own current semantic storage.
        tracing::info!("Engram memory provider selected; skipping SQLite KG backfill");

        // Belief Network stores (Phase B-1/B-2 + B-5 HTTP surface + B-6 observatory).
        //
        // Two layers of gating:
        //   1. Trait store handles come from the selected memory provider.
        //   2. We only park them on `AppState` for the HTTP layer when
        //      `execution.memory.beliefNetwork.enabled = true` — so the
        //      `/api/beliefs/*`, `/api/contradictions/*`, and
        //      `/api/belief-network/*` endpoints cleanly return 503/empty
        //      when the feature is off.
        //
        // The sleep-time worker block below still gets to consume the
        // handles either way (it has its own internal enable flag).

        // Sleep-time worker is trait-routed. Gates on the trait stores
        // (kg_store, episode_store, memory_store, procedure_store,
        // compaction_store) all wired above from the selected provider.
        // Conversation store is always SQLite-backed (per design) and
        // built unconditionally above.
        let (sleep_time_worker, belief_network_activity) = match (
            kg_store.as_ref(),
            episode_store_for_state.as_ref(),
            memory_store.as_ref(),
            procedure_store_for_state.as_ref(),
            compaction_store.as_ref(),
        ) {
            (Some(kgs), Some(eps), Some(mems), Some(prs), Some(compstore)) => {
                let conflict_interval_hours = settings
                    .get_execution_settings()
                    .map(|s| s.memory.conflict_resolver_interval_hours)
                    .unwrap_or(24);
                // `belief_network_cfg` is already defined in the outer
                // scope above (used for HTTP-store gating). Reuse it here.
                let memory_services =
                    gateway_memory::MemoryServices::new(gateway_memory::MemoryServicesConfig {
                        agent_id: "root".to_string(),
                        interval: std::time::Duration::from_secs(60 * 60),
                        llm_factory: memory_llm_factory.clone(),
                        kg_store: kgs.clone(),
                        episode_store: eps.clone(),
                        memory_store: mems.clone(),
                        compaction_store: compstore.clone(),
                        procedure_store: prs.clone(),
                        message_store: messages.clone(),
                        embedding_client: embedding_client.clone(),
                        kg_decay_config: recall_config.kg_decay.clone(),
                        conflict_resolver_interval: std::time::Duration::from_secs(
                            conflict_interval_hours as u64 * 3600,
                        ),
                        decay_config: gateway_memory::sleep::DecayConfig::default(),
                        belief_store: belief_store_raw.clone(),
                        belief_network_enabled: belief_network_cfg.enabled,
                        belief_network_interval: std::time::Duration::from_secs(
                            belief_network_cfg.interval_hours as u64 * 3600,
                        ),
                        belief_contradiction_store: belief_contradiction_store_raw.clone(),
                        belief_contradiction_neighborhood_prefix_depth: belief_network_cfg
                            .neighborhood_prefix_depth,
                        belief_contradiction_budget_per_cycle: belief_network_cfg
                            .contradiction_budget_per_cycle,
                        belief_fact_confidence_drop_threshold: belief_network_cfg
                            .fact_confidence_drop_threshold,
                        // Phase H-3: hierarchical memory. Reads
                        // execution.memory.hierarchy from settings; falls
                        // back to a disabled HierarchySettings::default()
                        // when settings aren't available so the daemon
                        // boots cleanly even with a partial config.
                        hierarchy_enabled: settings
                            .get_execution_settings()
                            .map(|s| s.memory.hierarchy.enabled)
                            .unwrap_or(false),
                        hierarchy_interval: std::time::Duration::from_secs(
                            settings
                                .get_execution_settings()
                                .map(|s| s.memory.hierarchy.interval_hours)
                                .unwrap_or(24) as u64
                                * 3600,
                        ),
                        hierarchy_max_layers: settings
                            .get_execution_settings()
                            .map(|s| s.memory.hierarchy.max_layers)
                            .unwrap_or(4),
                        hierarchy_cluster_target_size: settings
                            .get_execution_settings()
                            .map(|s| s.memory.hierarchy.cluster_target_size)
                            .unwrap_or(20),
                        hierarchy_inter_cluster_relation_threshold: settings
                            .get_execution_settings()
                            .map(|s| s.memory.hierarchy.inter_cluster_relation_threshold)
                            .unwrap_or(3),
                        hierarchy_llm_budget_per_cycle: settings
                            .get_execution_settings()
                            .map(|s| s.memory.hierarchy.llm_budget_per_cycle)
                            .unwrap_or(50),
                        // MEM-001 Part A — defaults today. The struct
                        // lives in `gateway-memory::sleep` and can be
                        // overridden once `settings.memory.contradiction`
                        // is added to the public settings surface.
                        contradiction_propagation_config:
                            gateway_memory::sleep::ContradictionPropagationConfig::default(),
                    });
                (
                    Some(memory_services.sleep_time_worker.clone()),
                    Some(memory_services.belief_network_activity.clone()),
                )
            }
            _ => (None, None),
        };

        // Create hook registry
        let hook_registry = Arc::new(HookRegistry::new(event_bus.clone()));

        // Create delegation registry
        let delegation_registry = Arc::new(DelegationRegistry::new());

        // Create plugin manager
        let plugin_manager = Arc::new(gateway_bridge::PluginManager::new(
            paths.plugins_dir(),
            bridge_registry.clone(),
            bridge_outbox.clone(),
            None, // bus is set later by server.start()
        ));

        Self {
            agents,
            skills,
            provider_service,
            mcp_service,
            runtime,
            event_bus,
            hook_registry: Some(hook_registry),
            messages,
            session_meta,
            checkpoints,
            autonomy,
            slim_logs,
            trace_analytics,
            delegation_registry,
            settings,
            log_service,
            state_service,
            durable_work_store,
            durable_work_transport,
            connector_registry,
            bridge_registry,
            bridge_outbox,
            bridge_bus: None,     // Set by server.start() before router creation
            cron_scheduler: None, // Initialized by server.start()
            session_archiver: Some(session_archiver),
            sleep_time_worker,
            compaction_store,
            plugin_manager,
            model_registry,
            embedding_service,
            paths,
            vault_dir,
            memory_store,
            goal_store: engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.goal_store.clone()),
            distillation_repo,
            distiller: distiller_ref,
            episode_store: episode_store_for_state,
            wiki_store: wiki_store_for_state,
            procedure_store: procedure_store_for_state,
            kg_episode_store,
            kg_store,
            governance_health: engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.governance_health.clone()),
            ingestion_queue,
            ingestion_backpressure,
            advertiser: discovery::noop(),
            advertise_handle: Arc::new(std::sync::Mutex::new(None)),
            belief_store: belief_store_for_http,
            belief_contradiction_store: belief_contradiction_store_for_http,
            belief_network_activity,
        }
    }

    /// Create a minimal state without execution runner (for testing).
    pub fn minimal(vault_dir: PathBuf) -> Self {
        let paths = Arc::new(VaultPaths::new(vault_dir.clone()));
        if let Err(e) = paths.ensure_dirs_exist() {
            tracing::warn!("Failed to create vault directories: {}", e);
        }
        if let Err(e) = gateway_services::seed_default_ward_archetypes(&paths) {
            tracing::warn!("Failed to seed editable ward archetypes: {}", e);
        }
        if let Err(e) = paths.migrate_legacy_layout() {
            tracing::warn!("Failed to migrate legacy vault layout: {}", e);
        }
        let agents_dir = paths.agents_dir();
        let skills_roots = paths.skills_dirs();
        let event_bus = Arc::new(EventBus::new());

        // Initialize SQLite database for conversation persistence
        let db_manager = Arc::new(
            DatabaseManager::new(paths.clone())
                .expect("Failed to initialize conversation database"),
        );
        let log_service = Arc::new(LogService::new(db_manager.clone()));
        let bridge_outbox = Arc::new(gateway_bridge::OutboxRepository::new(db_manager.clone()));
        let state_service = Arc::new(StateService::new(db_manager.clone()));
        let durable_work_store: Arc<dyn WorkStore> = Arc::new(SqliteWorkStore::new(db_manager));
        let durable_work_transport = Arc::new(gateway_bus::LocalWorkTransport::new());
        let settings = Arc::new(SettingsService::new(paths.clone()));
        state_service.set_surface_persistence_enabled(
            settings
                .get_presentation_settings()
                .map(|value| value.persist_surfaces)
                .unwrap_or(false),
        );
        let engram_store_bundle = persistence_factory::build_engram_store_bundle(
            paths.as_ref(),
            &gateway_memory::MemoryProviderSettings::default(),
            None,
        )
        .expect("Failed to initialize Engram memory provider for minimal state");

        // Create connector registry
        let connector_service = ConnectorService::new(paths.clone());
        let connector_registry = Arc::new(ConnectorRegistry::new(connector_service));

        // Create bridge registry
        let bridge_registry = Arc::new(gateway_bridge::BridgeRegistry::new());

        // Create plugin manager
        let plugin_manager = Arc::new(gateway_bridge::PluginManager::new(
            paths.plugins_dir(),
            bridge_registry.clone(),
            bridge_outbox.clone(),
            None, // bus is set later by server.start()
        ));

        let memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>> =
            Some(engram_store_bundle.memory_store.clone());
        let episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>> =
            Some(engram_store_bundle.episode_store.clone());
        let wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>> =
            Some(engram_store_bundle.wiki_store.clone());
        let procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>> =
            Some(engram_store_bundle.procedure_store.clone());
        let kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>> =
            Some(engram_store_bundle.kg_episode_store.clone());
        let kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>> =
            Some(engram_store_bundle.kg_store.clone());

        let (messages, session_meta, checkpoints, autonomy, slim_logs, trace_analytics) =
            build_conversation_stores(&paths)
                .expect("Failed to initialize conversation/trace stores");

        Self {
            messages,
            session_meta,
            checkpoints,
            autonomy,
            slim_logs,
            trace_analytics,
            agents: Arc::new(AgentService::new(agents_dir)),
            skills: Arc::new(SkillService::with_roots(skills_roots)),
            provider_service: Arc::new(ProviderService::new(paths.clone())),
            mcp_service: Arc::new(McpService::new(paths.clone())),
            runtime: Arc::new(RuntimeService::new(event_bus.clone())),
            event_bus,
            hook_registry: None,
            delegation_registry: Arc::new(DelegationRegistry::new()),
            settings,
            log_service,
            state_service,
            durable_work_store,
            durable_work_transport,
            connector_registry,
            bridge_registry,
            bridge_outbox,
            bridge_bus: None,
            cron_scheduler: None,
            session_archiver: None,
            sleep_time_worker: None,
            compaction_store: None,
            model_registry: Arc::new(ModelRegistry::load()),
            embedding_service: Arc::new(
                EmbeddingService::with_config(paths.clone(), Default::default())
                    .expect("default EmbeddingService must build"),
            ),
            plugin_manager,
            paths,
            vault_dir,
            memory_store,
            goal_store: Some(engram_store_bundle.goal_store.clone()),
            distillation_repo: None,
            distiller: None,
            episode_store,
            wiki_store,
            procedure_store,
            kg_episode_store,
            kg_store,
            governance_health: Some(engram_store_bundle.governance_health.clone()),
            ingestion_queue: None,
            ingestion_backpressure: None,
            advertiser: discovery::noop(),
            advertise_handle: Arc::new(std::sync::Mutex::new(None)),
            belief_store: None,
            belief_contradiction_store: None,
            belief_network_activity: None,
        }
    }

    /// Build an actor-filtered context capability catalog for HTTP/API
    /// inspection. Production uses the execution runner's live dependency set;
    /// minimal test state falls back to the same `ExecutorBuilder` with the
    /// services retained directly on `AppState`.
    pub fn context_capability_catalog(
        &self,
        actor_kind: gateway_execution::invoke::RuntimeActorKind,
        session_id: Option<String>,
        agent_id: Option<String>,
    ) -> agent_runtime::ContextCapabilityCatalog {
        let tool_settings = self.settings.get_tool_settings().unwrap_or_default();

        if let Some(runner) = self.runtime.runner() {
            return runner.context_capability_catalog(
                actor_kind,
                tool_settings,
                session_id,
                agent_id,
            );
        }

        capability_catalog::ToolCatalog {
            paths: self.paths.clone(),
            state_service: self.state_service.clone(),
            messages: self.messages.clone(),
            model_registry: self.model_registry.clone(),
            memory_store: self.memory_store.clone(),
            kg_store: self.kg_store.clone(),
            ingestion_queue: self.ingestion_queue.clone(),
            kg_episode_store: self.kg_episode_store.clone(),
            goal_store: self.goal_store.clone(),
            procedure_store: self.procedure_store.clone(),
            connector_provider: self.connector_resource_provider(),
        }
        .build(actor_kind, tool_settings, session_id, agent_id)
    }

    /// Build the HTTP-facing capability catalog and enrich the first-party tool
    /// snapshot with read-only resource/context provider metadata. This never
    /// registers or executes tools; it only reports discoverable surfaces.
    pub async fn context_capability_catalog_with_resources(
        &self,
        actor_kind: gateway_execution::invoke::RuntimeActorKind,
        session_id: Option<String>,
        agent_id: Option<String>,
    ) -> agent_runtime::ContextCapabilityCatalog {
        let catalog = self.context_capability_catalog(actor_kind, session_id, agent_id);
        capability_catalog::ResourceCatalog {
            local: capability_catalog::LocalProviderStatus {
                memory_store: self.memory_store.is_some(),
                kg_store: self.kg_store.is_some(),
                ingestion_queue: self.ingestion_queue.is_some(),
                compaction_store: self.compaction_store.is_some(),
                belief_store: self.belief_store.is_some(),
            },
            mcp_service: self.mcp_service.clone(),
            connector_provider: self.connector_resource_provider(),
        }
        .enrich(catalog)
        .await
    }

    fn connector_resource_provider(
        &self,
    ) -> Option<Arc<dyn agent_primitives::ConnectorResourceProvider>> {
        let http_provider: Option<Arc<dyn agent_primitives::ConnectorResourceProvider>> =
            Some(Arc::new(gateway_execution::GatewayResourceProvider::new(
                self.connector_registry.clone(),
            )));
        let bridge_provider: Option<Arc<dyn agent_primitives::ConnectorResourceProvider>> =
            Some(Arc::new(gateway_bridge::BridgeResourceProvider::new(
                self.bridge_registry.clone(),
                self.bridge_outbox.clone(),
            )));

        Some(Arc::new(gateway_execution::CompositeResourceProvider::new(
            http_provider,
            bridge_provider,
        )))
    }

    /// Create with custom components.
    #[allow(clippy::too_many_arguments)]
    pub fn with_components(
        agents: Arc<AgentService>,
        skills: Arc<SkillService>,
        provider_service: Arc<ProviderService>,
        mcp_service: Arc<McpService>,
        runtime: Arc<RuntimeService>,
        event_bus: Arc<EventBus>,
        log_service: Arc<LogService<DatabaseManager>>,
        state_service: Arc<StateService<DatabaseManager>>,
        connector_registry: Arc<ConnectorRegistry>,
        paths: SharedVaultPaths,
    ) -> Self {
        let vault_dir = paths.vault_dir().clone();
        let settings = Arc::new(SettingsService::new(paths.clone()));
        state_service.set_surface_persistence_enabled(
            settings
                .get_presentation_settings()
                .map(|value| value.persist_surfaces)
                .unwrap_or(false),
        );
        let engram_store_bundle = persistence_factory::build_engram_store_bundle(
            paths.as_ref(),
            &gateway_memory::MemoryProviderSettings::default(),
            None,
        )
        .expect("Failed to initialize Engram memory provider for component state");
        let memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>> =
            Some(engram_store_bundle.memory_store.clone());
        let episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>> =
            Some(engram_store_bundle.episode_store.clone());
        let wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>> =
            Some(engram_store_bundle.wiki_store.clone());
        let procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>> =
            Some(engram_store_bundle.procedure_store.clone());
        let kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>> =
            Some(engram_store_bundle.kg_episode_store.clone());
        let kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>> =
            Some(engram_store_bundle.kg_store.clone());

        // Create bridge registry and outbox
        let bridge_registry = Arc::new(gateway_bridge::BridgeRegistry::new());
        let db_manager = Arc::new(
            DatabaseManager::new(paths.clone())
                .expect("Failed to initialize database for bridge outbox and durable work"),
        );
        let bridge_outbox = Arc::new(gateway_bridge::OutboxRepository::new(db_manager.clone()));
        let durable_work_store: Arc<dyn WorkStore> = Arc::new(SqliteWorkStore::new(db_manager));
        let durable_work_transport = Arc::new(gateway_bus::LocalWorkTransport::new());

        // Create plugin manager
        let plugin_manager = Arc::new(gateway_bridge::PluginManager::new(
            paths.plugins_dir(),
            bridge_registry.clone(),
            bridge_outbox.clone(),
            None, // bus is set later by server.start()
        ));

        let (messages, session_meta, checkpoints, autonomy, slim_logs, trace_analytics) =
            build_conversation_stores(&paths)
                .expect("Failed to initialize conversation/trace stores");

        Self {
            agents,
            skills,
            provider_service,
            mcp_service,
            runtime,
            event_bus,
            hook_registry: None,
            messages,
            session_meta,
            checkpoints,
            autonomy,
            slim_logs,
            trace_analytics,
            delegation_registry: Arc::new(DelegationRegistry::new()),
            settings,
            log_service,
            state_service,
            durable_work_store,
            durable_work_transport,
            connector_registry,
            bridge_registry,
            bridge_outbox,
            bridge_bus: None,
            cron_scheduler: None,
            session_archiver: None,
            sleep_time_worker: None,
            compaction_store: None,
            model_registry: Arc::new(ModelRegistry::load()),
            embedding_service: Arc::new(
                EmbeddingService::with_config(paths.clone(), Default::default())
                    .expect("default EmbeddingService must build"),
            ),
            plugin_manager,
            paths,
            vault_dir,
            memory_store,
            goal_store: Some(engram_store_bundle.goal_store.clone()),
            distillation_repo: None,
            distiller: None,
            episode_store,
            wiki_store,
            procedure_store,
            kg_episode_store,
            kg_store,
            governance_health: Some(engram_store_bundle.governance_health.clone()),
            ingestion_queue: None,
            ingestion_backpressure: None,
            advertiser: discovery::noop(),
            advertise_handle: Arc::new(std::sync::Mutex::new(None)),
            belief_store: None,
            belief_contradiction_store: None,
            belief_network_activity: None,
        }
    }

    /// Create with hook registry.
    pub fn with_hook_registry(mut self, hook_registry: Arc<HookRegistry>) -> Self {
        self.hook_registry = Some(hook_registry);
        self
    }

    /// Reconcile the embedding client health against current settings.
    ///
    /// Runs at boot (from `GatewayServer::start`) and performs two things:
    ///
    /// 1. Pre-emptive Ollama ping — surfaces unreachability in `Health`
    ///    immediately instead of waiting for the periodic health loop.
    /// 2. Spawns the periodic health-check loop (60s tick).
    ///
    /// Semantic memory/knowledge indexing lives behind Engram. The old SQLite
    /// vec-index rebuild path is intentionally not a production fallback.
    pub async fn reconcile_embeddings_at_boot(&self) {
        self.embedding_service.preflight().await;

        if self.embedding_service.needs_reindex() {
            let current_dim = self.embedding_service.dimensions();
            if let Err(e) = self.embedding_service.mark_indexed(current_dim) {
                tracing::warn!("mark_indexed failed after embedding preflight: {e}");
            } else {
                tracing::info!(
                    dim = current_dim,
                    "Embedding marker updated; Engram owns semantic index maintenance"
                );
            }
        }

        let _handle = self.embedding_service.clone().start_health_loop();
        // JoinHandle intentionally dropped — loop lives for the process
        // lifetime; daemon shutdown drops the runtime.
    }

    /// Seed default agents and other initial data.
    ///
    /// This should be called after creating the state to set up default subagents
    /// that can be delegated to.
    pub async fn seed_defaults(&self) {
        // Get default provider ID
        let providers = self.provider_service.list().unwrap_or_default();
        let selected = gateway_services::select_provider(&providers, None);
        let default_provider_id = selected
            .and_then(|p| p.id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Resolve default model from default provider (first model in list)
        let default_model = selected
            .map(|p| p.default_model().to_string())
            .unwrap_or_else(|| "gpt-4o".to_string());

        // Seed default agents from bundled templates (configs + AGENTS.md instructions)
        let agent_template =
            gateway_templates::Templates::get("default_agents.json").map(|f| f.data.to_vec());
        if let Err(e) = self
            .agents
            .seed_default_agents(
                &default_provider_id,
                &default_model,
                agent_template.as_deref(),
                |name| {
                    let path = format!("agents/{}.md", name);
                    gateway_templates::Templates::get(&path)
                        .map(|f| String::from_utf8_lossy(&f.data).to_string())
                },
            )
            .await
        {
            tracing::warn!("Failed to seed default agents: {}", e);
        }

        // Seed default skills from bundled templates if skills dir is empty
        self.seed_default_skills();

        // Seed default cron jobs (idempotent on job id) so first-run
        // installs ship with the bundled cleanup schedule wired up.
        self.seed_default_cron().await;

        // Seed default policies from bundled template if no policies exist
        self.seed_default_policies().await;

        // Preload skills into cache
        if let Err(e) = self.skills.preload().await {
            tracing::warn!("Failed to preload skills: {}", e);
        }

        // Seed required workspace structure. Runtime environments are created
        // only by an explicit tool/runtime action, never during startup.
        self.ensure_runtime_environments().await;

        // Discover and start plugins
        self.discover_and_start_plugins().await;
    }

    /// Discover and start all enabled plugins.
    async fn discover_and_start_plugins(&self) {
        tracing::info!("Discovering plugins...");

        match self.plugin_manager.discover().await {
            Ok(discovered) => {
                if discovered.is_empty() {
                    tracing::info!("No plugins discovered");
                } else {
                    tracing::info!(
                        "Discovered {} plugin(s): {:?}",
                        discovered.len(),
                        discovered
                    );

                    // Start all enabled plugins
                    self.plugin_manager.start_all().await;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to discover plugins: {}", e);
            }
        }
    }

    /// Seed default skills from bundled templates if skills directory is empty.
    fn seed_default_skills(&self) {
        let skills_dir = self.paths.vault_dir().join("skills");

        // Only seed if skills dir is empty or doesn't exist
        let has_skills = skills_dir.exists()
            && std::fs::read_dir(&skills_dir)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false);

        if has_skills {
            tracing::debug!("Skills directory not empty, skipping seed");
            return;
        }

        tracing::info!("Seeding default skills from bundled templates");
        std::fs::create_dir_all(&skills_dir).ok();

        // Iterate all embedded files under skills/
        for path in gateway_templates::Templates::iter() {
            let path_str = path.as_ref();
            if !path_str.starts_with("skills/") {
                continue;
            }

            // path_str is like "skills/coding/SKILL.md" or "skills/yfinance-market-analysis/scripts/run.py"
            let dest = self.paths.vault_dir().join(path_str);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).ok();
            }

            if let Some(file) = gateway_templates::Templates::get(path_str) {
                if let Err(e) = std::fs::write(&dest, &file.data) {
                    tracing::warn!("Failed to seed skill file {}: {}", path_str, e);
                }
            }
        }

        let count = std::fs::read_dir(&skills_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        tracing::info!("Seeded {} default skills", count);
    }

    /// Seed default cron jobs from bundled `default_cron.json` template.
    ///
    /// Each ID is seeded **at most once per vault**: the first time we see
    /// it, we create the job (or migrate a pre-existing one) and record the
    /// ID in `<vault>/config/seeded-defaults.json`. Subsequent boots skip
    /// any ID already in the registry, so deletes the user makes through
    /// the UI stick across daemon restarts.
    async fn seed_default_cron(&self) {
        let template_bytes = match gateway_templates::Templates::get("default_cron.json") {
            Some(file) => file.data.to_vec(),
            None => {
                tracing::debug!(
                    "seed_default_cron: bundled `default_cron.json` not found, skipping"
                );
                return;
            }
        };

        let requests: Vec<gateway_cron::CreateCronJobRequest> =
            match serde_json::from_slice(&template_bytes) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("seed_default_cron: failed to parse default_cron.json: {e}");
                    return;
                }
            };

        if requests.is_empty() {
            tracing::debug!("seed_default_cron: no entries in default_cron.json");
            return;
        }

        let cron_service = gateway_cron::CronService::new(self.paths.clone());
        let seeded =
            seeded_defaults::seed_cron_with_registry(&self.paths, &cron_service, requests).await;

        if seeded > 0 {
            tracing::info!(seeded, "seed_default_cron: completed");
        }
    }

    /// Seed default policies from bundled template if no policies/corrections exist.
    async fn seed_default_policies(&self) {
        // Route through the trait surface so Engram receives the same
        // default policy seed data as the rest of the runtime.
        let memory_store = match &self.memory_store {
            Some(s) => s,
            None => {
                tracing::warn!(
                    "seed_default_policies: memory_store is None — refusing to seed. \
                     Check persistence_factory output."
                );
                return;
            }
        };

        // Check if any correction facts already exist for the root agent.
        let existing = match memory_store
            .list_memory_facts(Some("root"), Some("correction"), None, 1, 0)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    "seed_default_policies: existence check failed ({e}); \
                     proceeding as if empty (may produce duplicates if policies \
                     are already present)."
                );
                Vec::new()
            }
        };
        if !existing.is_empty() {
            tracing::debug!(
                existing_count = existing.len(),
                "seed_default_policies: policies already present for root/correction — skipping"
            );
            return;
        }

        let template = match gateway_templates::Templates::get("default_policies.json") {
            Some(f) => f.data.to_vec(),
            None => {
                tracing::warn!(
                    "seed_default_policies: bundled `default_policies.json` template \
                     missing from gateway-templates — nothing to seed."
                );
                return;
            }
        };

        let policies: Vec<serde_json::Value> = match serde_json::from_slice(&template) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("seed_default_policies: failed to parse default_policies.json: {e}");
                return;
            }
        };

        let total = policies.len();
        let now = chrono::Utc::now().to_rfc3339();
        let mut count = 0usize;
        let mut skipped_empty = 0usize;
        let mut errors: Vec<(String, String)> = Vec::new();

        for policy in &policies {
            let category = policy["category"].as_str().unwrap_or("correction");
            let key = policy["key"].as_str().unwrap_or_default();
            let content = policy["content"].as_str().unwrap_or_default();
            let confidence = policy["confidence"].as_f64().unwrap_or(1.0);
            let pinned = policy["pinned"].as_bool().unwrap_or(true);

            if key.is_empty() || content.is_empty() {
                skipped_empty += 1;
                continue;
            }

            let fact_value = zbot_stores_domain::MemoryFact {
                id: format!("policy-{}", uuid::Uuid::new_v4()),
                session_id: None,
                agent_id: "root".to_string(),
                scope: "agent".to_string(),
                category: category.to_string(),
                key: key.to_string(),
                content: content.to_string(),
                confidence,
                mention_count: 5,
                source_summary: Some("Default policy".to_string()),
                ward_id: "__global__".to_string(),
                contradicted_by: None,
                created_at: now.clone(),
                updated_at: now.clone(),
                expires_at: None,
                valid_from: None,
                valid_until: None,
                superseded_by: None,
                pinned,
                epistemic_class: Some("current".to_string()),
                source_episode_id: None,
                source_ref: None,
                embedding: None,
                last_accessed: None,
            };

            match memory_store.upsert_typed_fact(fact_value, None).await {
                Ok(()) => count += 1,
                Err(e) => errors.push((key.to_string(), e.to_string())),
            }
        }

        if !errors.is_empty() {
            for (key, e) in &errors {
                tracing::warn!(policy_key = %key, error = %e, "seed_default_policies: upsert failed");
            }
        }

        tracing::info!(
            total = total,
            seeded = count,
            skipped_empty = skipped_empty,
            failed = errors.len(),
            "seed_default_policies: completed"
        );
    }

    /// Ensure the required workspace structure exists without creating optional
    /// Python or Node runtime directories on every startup.
    async fn ensure_runtime_environments(&self) {
        self.ensure_wards_dir();
    }

    /// Ensure the wards root exists. Its catalog is created safely by the
    /// ward tool on first use.
    fn ensure_wards_dir(&self) {
        let wards_dir = self.vault_dir.join("wards");
        if let Err(error) = agent_tools::ensure_ward_catalog(&wards_dir) {
            tracing::error!(%error, root = %wards_dir.display(), "failed to initialize wards root");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::hooks::HookRegistry;
    use tempfile::TempDir;

    fn make_temp_state() -> (TempDir, AppState) {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();
        let state = AppState::minimal(dir.path().to_path_buf());
        (dir, state)
    }

    #[test]
    fn minimal_app_state_wires_required_components() {
        let (_dir, state) = make_temp_state();
        assert!(state.hook_registry.is_none());
        assert!(state.cron_scheduler.is_none());
        assert!(state.session_archiver.is_none());
        assert!(state.bridge_bus.is_none());
        assert!(state.memory_store.is_some());
        assert!(state.episode_store.is_some());
        assert!(state.wiki_store.is_some());
        assert!(state.procedure_store.is_some());
        assert!(state.kg_store.is_some());
        assert!(state.kg_episode_store.is_some());
    }

    #[test]
    fn fresh_database_and_vault_bootstrap_can_create_a_coding_ward() {
        let dir = TempDir::new().unwrap();
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());

        let _state = AppState::minimal(dir.path().to_path_buf());
        let paths = VaultPaths::new(dir.path().to_path_buf());
        assert!(paths.conversations_db().is_file());
        let created = gateway_services::create_ward_from_archetype(
            &paths,
            "fresh-code",
            Some(agent_primitives::WardArchetypeId::Coding),
        )
        .unwrap();

        assert_eq!(created.archetype, agent_primitives::WardArchetypeId::Coding);
        let mut entries = std::fs::read_dir(&created.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(
            entries,
            ["AGENTS.md", "fresh-code.md", "log.md", "ward-conf.yaml"]
        );
        for lazy_directory in ["src", "tests", "docs", "scripts", "artifacts", ".zbot"] {
            assert!(!created.path.join(lazy_directory).exists());
        }
    }

    #[test]
    fn with_hook_registry_wires_the_optional_field() {
        let (_dir, state) = make_temp_state();
        let event_bus = Arc::new(EventBus::new());
        let registry = Arc::new(HookRegistry::new(event_bus));
        let state = state.with_hook_registry(registry);
        assert!(state.hook_registry.is_some());
    }

    #[test]
    fn ensure_wards_dir_creates_only_plain_root_catalog() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();
        let wards = state.vault_dir.join("wards");
        assert_eq!(std::fs::read_dir(&wards).unwrap().count(), 1);
        assert_eq!(
            std::fs::read_to_string(wards.join("index.md")).unwrap(),
            "# Wards\n"
        );
    }

    #[test]
    fn ensure_wards_dir_is_idempotent_and_preserves_root_index() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();
        let index = state.vault_dir.join("wards").join("index.md");

        std::fs::write(&index, "# My catalog\n").unwrap();
        state.ensure_wards_dir();
        assert_eq!(std::fs::read_to_string(index).unwrap(), "# My catalog\n");
    }

    #[test]
    fn ensure_wards_dir_does_not_seed_legacy_wards() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();
        state.ensure_wards_dir();
        assert!(!state.vault_dir.join("wards/scratch").exists());
        assert!(!state.vault_dir.join("wards/wiki").exists());
    }

    #[test]
    fn seed_default_skills_is_no_op_when_skills_dir_has_content() {
        let (_dir, state) = make_temp_state();
        let skills_dir = state.paths.vault_dir().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("sentinel.md"), "user").unwrap();

        state.seed_default_skills();
        assert!(skills_dir.join("sentinel.md").exists());
    }

    #[test]
    fn seed_default_skills_populates_empty_dir_from_templates() {
        let (_dir, state) = make_temp_state();
        let skills_dir = state.paths.vault_dir().join("skills");
        if skills_dir.exists() {
            std::fs::remove_dir_all(&skills_dir).unwrap();
        }
        state.seed_default_skills();
        assert!(skills_dir.is_dir());
    }

    #[tokio::test]
    async fn seed_default_cron_inserts_bundled_jobs_into_registry() {
        let (_dir, state) = make_temp_state();
        state.seed_default_cron().await;

        let registry_path = state.paths.seeded_defaults();
        assert!(registry_path.exists());
    }

    #[tokio::test]
    async fn seed_default_cron_is_idempotent_across_calls() {
        let (_dir, state) = make_temp_state();
        state.seed_default_cron().await;
        state.seed_default_cron().await;
    }

    #[tokio::test]
    async fn seed_default_policies_skips_when_existing_corrections_present() {
        let (_dir, state) = make_temp_state();
        state.seed_default_policies().await;
        state.seed_default_policies().await;
    }

    #[tokio::test]
    async fn discover_and_start_plugins_handles_missing_plugin_dir() {
        let (_dir, state) = make_temp_state();
        state.discover_and_start_plugins().await;
    }

    #[tokio::test]
    async fn ensure_runtime_environments_creates_workspace_without_optional_envs() {
        let (_dir, state) = make_temp_state();
        state.ensure_runtime_environments().await;

        assert!(state.vault_dir.join("wards").join("index.md").is_file());
        assert!(!state.vault_dir.join("wards").join("scratch").exists());
        assert!(!state.vault_dir.join("wards").join(".node_env").exists());
    }

    #[tokio::test]
    async fn seed_defaults_runs_to_completion() {
        let (_dir, state) = make_temp_state();
        state.seed_defaults().await;
    }

    #[tokio::test]
    async fn new_app_state_initialises_full_constructor_path() {
        let dir = TempDir::new().unwrap();
        let state = AppState::new(dir.path().to_path_buf());
        assert!(state.memory_store.is_some());
        assert!(state.kg_store.is_some());
        assert!(state.distillation_repo.is_some());
        assert!(state.distiller.is_some());
        assert!(state.session_archiver.is_some());
        assert!(state.episode_store.is_some());
        assert!(state.kg_episode_store.is_some());
        assert!(state.cron_scheduler.is_none());
        assert!(state.bridge_bus.is_none());
    }

    #[tokio::test]
    async fn new_app_state_engram_provider_skips_sqlite_knowledge_db() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("settings.json"),
            r#"{
                "execution": {
                    "memory": {
                        "provider": {
                            "mode": "engram",
                            "engramPath": "engram",
                            "tenant": "agentzero"
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let state = AppState::new(dir.path().to_path_buf());

        assert!(state.memory_store.is_some());
        assert!(state.kg_store.is_some());
        assert!(state.episode_store.is_some());
        assert!(state.wiki_store.is_some());
        assert!(state.procedure_store.is_some());
        assert!(!dir.path().join("data").join("knowledge.db").exists());
        assert!(dir.path().join("data").join("engram").exists());
    }

    #[test]
    fn with_components_uses_supplied_handles() {
        let dir = TempDir::new().unwrap();
        let paths: SharedVaultPaths = Arc::new(VaultPaths::new(dir.path().to_path_buf()));
        let _ = paths.ensure_dirs_exist();
        let event_bus = Arc::new(EventBus::new());
        let agents = Arc::new(AgentService::new(paths.agents_dir()));
        let skills = Arc::new(SkillService::with_roots(paths.skills_dirs()));
        let provider_service = Arc::new(ProviderService::new(paths.clone()));
        let mcp_service = Arc::new(McpService::new(paths.clone()));
        let runtime = Arc::new(RuntimeService::new(event_bus.clone()));
        let db_manager = Arc::new(DatabaseManager::new(paths.clone()).expect("db manager"));
        let log_service = Arc::new(LogService::new(db_manager.clone()));
        let state_service = Arc::new(StateService::new(db_manager.clone()));
        let connector_service = ConnectorService::new(paths.clone());
        let connector_registry = Arc::new(ConnectorRegistry::new(connector_service));

        let state = AppState::with_components(
            agents,
            skills,
            provider_service,
            mcp_service,
            runtime,
            event_bus,
            log_service,
            state_service,
            connector_registry,
            paths.clone(),
        );

        assert_eq!(state.vault_dir, *paths.vault_dir());
        assert!(state.memory_store.is_some());
        assert!(state.episode_store.is_some());
        assert!(state.wiki_store.is_some());
        assert!(state.procedure_store.is_some());
    }
}
