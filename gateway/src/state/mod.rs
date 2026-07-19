//! # Application State
//!
//! Shared state for the gateway application.

pub(crate) mod persistence_factory;
mod seeded_defaults;

use crate::connectors::{ConnectorRegistry, ConnectorService};
use crate::cron::CronScheduler;
use crate::events::EventBus;
use crate::execution::{DelegationRegistry, MemoryRecall, SessionArchiver, SessionDistiller};
use crate::hooks::HookRegistry;
use crate::services::{
    AgentService, McpService, ModelRegistry, ProviderService, RuntimeService, SettingsService,
    SharedVaultPaths, SkillService, VaultPaths,
};
use agent_primitives::connectors::{CapabilityInfo, ConnectorInfo, ResourceInfo};
use agent_runtime::llm::EmbeddingClient;
use agent_runtime::{
    ContextActorKind, ContextCapability, ContextCapabilityCatalog, ContextCapabilityHealth,
    ContextCapabilityKind, ContextCostHint, ContextLatencyHint, ContextRiskLevel,
    ContextSideEffects,
};
use api_logs::LogService;
use execution_state::StateService;
use gateway_services::{EmbeddingService, WardProvenance, WardUsage};
use std::collections::BTreeSet;
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
    pub memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>>,

    /// Backend-neutral active goals used for intent boost and the goal tool.
    pub goal_store: Option<Arc<dyn zbot_stores_traits::GoalStore>>,

    /// Distillation repository for tracking distillation run outcomes.
    pub distillation_repo: Option<Arc<DistillationRepository>>,

    /// Session distiller for triggering on-demand distillation (e.g., backfill).
    pub distiller: Option<Arc<SessionDistiller>>,

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
    pub kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>>,

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
        // Create centralized vault paths
        let paths = Arc::new(VaultPaths::new(vault_dir.clone()));

        // Ensure required directories exist
        if let Err(e) = paths.ensure_dirs_exist() {
            tracing::warn!("Failed to create vault directories: {}", e);
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
        // SessionDistiller construction, so they can be wired with it).
        let early_memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>> = engram_store_bundle
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
        let kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>> = engram_store_bundle
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

        // Self-RAG retrieval gate (opt-in via `memory.queryGate.enabled` in
        // settings.json). Reads settings eagerly here so the gate is attached
        // before MemoryRecall is sealed in Arc below. When the block is
        // missing, disabled, or unreadable, the gate stays None and recall
        // behaves identically to pre-gate behavior.
        let query_gate_cfg: gateway_memory::QueryGateConfig =
            gateway_services::SettingsService::new(paths.clone())
                .load()
                .map(|s| s.execution.memory.query_gate.clone())
                .unwrap_or_default();
        if query_gate_cfg.enabled {
            let llm = Arc::new(gateway_memory::LlmQueryGate::new(
                memory_llm_factory.clone(),
            ));
            let gate = Arc::new(gateway_memory::QueryGate::new(llm, query_gate_cfg.clone()));
            if let Some(recall) = memory_recall_inner.as_mut() {
                recall.set_query_gate(gate);
            }
            tracing::info!(
                "Memory query gate: enabled (model={:?}, max_subqueries={}, timeout_ms={})",
                query_gate_cfg.model_id,
                query_gate_cfg.max_subqueries,
                query_gate_cfg.timeout_ms,
            );
        } else {
            tracing::info!("Memory query gate: disabled");
        }

        // MMR diversity reranking (opt-in via `memory.mmr.enabled` in
        // settings.json). Default-disabled: when the block is missing or
        // `enabled = false`, recall is byte-for-byte identical to pre-MMR.
        // The config block is attached unconditionally so the runtime can
        // read the current values; only `enabled = true` triggers the
        // rerank step inside `recall_unified`.
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

        // SessionDistiller writes semantic artifacts through Engram-backed
        // trait stores. Conversation-linked run tracking still uses
        // conversations.db through DistillationRepository.
        let distiller: Option<Arc<SessionDistiller>> = if memory_store.is_some() {
            let mut distiller_inner = SessionDistiller::new(
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
        let distiller_ref: Option<Arc<SessionDistiller>> = distiller.clone();
        let max_parallel_agents = settings
            .get_execution_settings()
            .map(|s| s.max_parallel_agents)
            .unwrap_or(2);
        tracing::info!(max_parallel_agents, "Execution settings loaded");

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
        let belief_network_cfg = settings
            .get_execution_settings()
            .map(|s| s.memory.belief_network.clone())
            .unwrap_or_default();
        let belief_store_raw: Option<Arc<dyn zbot_stores::BeliefStore>> = engram_store_bundle
            .as_ref()
            .map(|bundle| bundle.belief_store.clone());
        let belief_contradiction_store_raw: Option<Arc<dyn zbot_stores::BeliefContradictionStore>> =
            engram_store_bundle
                .as_ref()
                .map(|bundle| bundle.belief_contradiction_store.clone());
        // HTTP surface only exposes the stores when the feature is on.
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
                let abstractions_interval_hours = settings
                    .get_execution_settings()
                    .map(|s| s.memory.corrections_abstractor_interval_hours)
                    .unwrap_or(24);
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
                        corrections_abstractor_interval: std::time::Duration::from_secs(
                            abstractions_interval_hours as u64 * 3600,
                        ),
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
        let state_service = Arc::new(StateService::new(db_manager));
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

        let memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>> =
            Some(engram_store_bundle.memory_store.clone());
        let episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>> =
            Some(engram_store_bundle.episode_store.clone());
        let wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>> =
            Some(engram_store_bundle.wiki_store.clone());
        let procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>> =
            Some(engram_store_bundle.procedure_store.clone());
        let kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>> =
            Some(engram_store_bundle.kg_episode_store.clone());
        let kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>> =
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
            settings: Arc::new(SettingsService::new(paths.clone())),
            log_service,
            state_service,
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

        let mut builder = gateway_execution::invoke::ExecutorBuilder::new(
            self.paths.vault_dir().clone(),
            tool_settings,
        )
        .with_actor_kind(actor_kind)
        .with_state_service(self.state_service.clone())
        .with_message_store(self.messages.clone())
        .with_model_registry(self.model_registry.clone());

        if let Some(store) = &self.memory_store {
            builder = builder.with_fact_store(store.clone());
        }
        if let Some(provider) = self.connector_resource_provider() {
            builder = builder.with_connector_provider(provider);
        }
        if let Some(store) = &self.kg_store {
            builder = builder.with_kg_store(store.clone());
        }
        if let (Some(queue), Some(kg_store), Some(kg_episode_store)) = (
            self.ingestion_queue.as_ref(),
            self.kg_store.as_ref(),
            self.kg_episode_store.as_ref(),
        ) {
            let adapter = Arc::new(
                gateway_execution::invoke::ingest_adapter::IngestionAdapter::new(
                    queue.clone(),
                    kg_episode_store.clone(),
                    kg_store.clone(),
                ),
            );
            builder = builder.with_ingestion_adapter(adapter);
        }
        if let Some(store) = &self.goal_store {
            let adapter = Arc::new(gateway_execution::invoke::goal_adapter::GoalAdapter::new(
                store.clone(),
            ));
            builder = builder.with_goal_adapter(adapter);
        }
        if let Some(store) = &self.procedure_store {
            builder = builder.with_procedure_store(store.clone());
        }

        let ward_usage = Arc::new(
            gateway_execution::invoke::ward_usage_adapter::WardUsageAdapter::new(Arc::new(
                WardUsage::new(self.paths.wards_dir()),
            )),
        );
        builder = builder
            .with_ward_usage(ward_usage)
            .with_steering_registry(Arc::new(agent_runtime::SteeringRegistry::new()))
            .with_agent_result_bus(Arc::new(
                gateway_execution::agent_pool::AgentResultBus::new(),
            ));

        builder.build_context_capability_catalog(session_id, agent_id)
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
        let mut catalog = self.context_capability_catalog(actor_kind, session_id, agent_id);

        let mut resource_capabilities = local_context_provider_capabilities(LocalProviderStatus {
            memory_store: self.memory_store.is_some(),
            kg_store: self.kg_store.is_some(),
            ingestion_queue: self.ingestion_queue.is_some(),
            compaction_store: self.compaction_store.is_some(),
            belief_store: self.belief_store.is_some(),
        });

        match self.mcp_service.list_summaries() {
            Ok(summaries) => resource_capabilities.extend(mcp_catalog_capabilities(summaries)),
            Err(error) => {
                tracing::warn!(%error, "Failed to enrich capability catalog with MCP metadata");
            }
        }

        if let Some(provider) = self.connector_resource_provider() {
            match provider.list_connectors().await {
                Ok(connectors) => {
                    resource_capabilities.extend(connector_catalog_capabilities(connectors));
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Failed to enrich capability catalog with connector resource metadata"
                    );
                }
            }
        }

        append_actor_visible_capabilities(&mut catalog, resource_capabilities);
        catalog
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
        let engram_store_bundle = persistence_factory::build_engram_store_bundle(
            paths.as_ref(),
            &gateway_memory::MemoryProviderSettings::default(),
            None,
        )
        .expect("Failed to initialize Engram memory provider for component state");
        let memory_store: Option<Arc<dyn zbot_stores::MemoryFactStore>> =
            Some(engram_store_bundle.memory_store.clone());
        let episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>> =
            Some(engram_store_bundle.episode_store.clone());
        let wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>> =
            Some(engram_store_bundle.wiki_store.clone());
        let procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>> =
            Some(engram_store_bundle.procedure_store.clone());
        let kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>> =
            Some(engram_store_bundle.kg_episode_store.clone());
        let kg_store: Option<Arc<dyn zbot_stores::KnowledgeGraphStore>> =
            Some(engram_store_bundle.kg_store.clone());

        // Create bridge registry and outbox
        let bridge_registry = Arc::new(gateway_bridge::BridgeRegistry::new());
        let bridge_outbox = {
            let db = Arc::new(
                DatabaseManager::new(paths.clone())
                    .expect("Failed to initialize database for bridge outbox"),
            );
            Arc::new(gateway_bridge::OutboxRepository::new(db))
        };

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
            settings: Arc::new(SettingsService::new(paths.clone())),
            log_service,
            state_service,
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
        let default_provider_id = self
            .provider_service
            .list()
            .ok()
            .and_then(|providers| {
                providers
                    .iter()
                    .find(|p| p.is_default)
                    .or_else(|| providers.first())
                    .and_then(|p| p.id.clone())
            })
            .unwrap_or_else(|| "default".to_string());

        // Resolve default model from default provider (first model in list)
        let default_model = self
            .provider_service
            .list()
            .ok()
            .and_then(|providers| {
                providers
                    .iter()
                    .find(|p| p.is_default)
                    .or_else(|| providers.first())
                    .and_then(|p| p.default_model().to_string().into())
            })
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

            let fact_value = serde_json::json!({
                "id": format!("policy-{}", uuid::Uuid::new_v4()),
                "session_id": null,
                "agent_id": "root",
                "scope": "agent",
                "category": category,
                "key": key,
                "content": content,
                "confidence": confidence,
                "mention_count": 5,
                "source_summary": "Default policy",
                "ward_id": "__global__",
                "contradicted_by": null,
                "created_at": now,
                "updated_at": now,
                "expires_at": null,
                "valid_from": null,
                "valid_until": null,
                "superseded_by": null,
                "pinned": pinned,
                "epistemic_class": "current",
                "source_episode_id": null,
                "source_ref": null,
            });

            match memory_store.upsert_typed_fact(fact_value, None).await {
                Ok(()) => count += 1,
                Err(e) => errors.push((key.to_string(), e)),
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

    /// Create the wards directory with scratch ward + wiki vault ward.
    ///
    /// The wiki ward is the Obsidian vault — it receives promoted content
    /// from producer-skill runs (book-reader, research archetypes) via the
    /// `wiki` skill. Its name is configurable via `settings.json →
    /// execution.wiki.wardName` (default `"wiki"`). We seed it at startup so
    /// delegated subagents (which cannot create wards) can just `use` it.
    fn ensure_wards_dir(&self) {
        let wards_dir = self.vault_dir.join("wards");
        let scratch_dir = wards_dir.join("scratch");

        if !scratch_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&scratch_dir) {
                tracing::warn!("Failed to create wards/scratch directory: {}", e);
            } else {
                tracing::info!(
                    "Created wards directory with scratch ward at {}",
                    wards_dir.display()
                );
            }
        }

        // Mark the bundled provenance so the curator never archives `scratch`.
        // Idempotent: re-marking on every boot just refreshes `created_by`.
        if let Err(e) = WardUsage::new(&wards_dir).mark_created("scratch", WardProvenance::Bundled)
        {
            tracing::warn!(error = %e, "ward_usage: failed to mark scratch as bundled");
        }

        let wiki_name = self
            .settings
            .load()
            .ok()
            .map(|s| s.execution.wiki.ward_name)
            .unwrap_or_else(|| "wiki".to_string());

        self.ensure_wiki_ward(&wards_dir, &wiki_name);
    }

    /// Create the wiki vault ward with canonical Obsidian tree + AGENTS.md marker.
    ///
    /// Idempotent — existing content is preserved. The marker
    /// `<!-- obsidian-vault -->` in AGENTS.md lets the `wiki` skill discover
    /// this ward via `ward(action="list")` regardless of the configured name.
    fn ensure_wiki_ward(&self, wards_dir: &std::path::Path, wiki_name: &str) {
        let wiki_dir = wards_dir.join(wiki_name);
        if let Err(e) = std::fs::create_dir_all(&wiki_dir) {
            tracing::warn!("Failed to create wiki ward directory: {}", e);
            return;
        }

        // Canonical Obsidian vault top-level folders.
        let vault_folders = [
            "00_Inbox",
            "10_Journal/Daily",
            "10_Journal/Weekly",
            "20_Projects",
            "30_Library/Books",
            "30_Library/Articles",
            "40_Research",
            "50_Resources",
            "60_Archive",
            "70_Assets/Knowledge_Graphs",
            "70_Assets/Images",
            "70_Assets/Documents",
            "_zztemplates",
        ];
        for folder in vault_folders {
            let _ = std::fs::create_dir_all(wiki_dir.join(folder));
        }

        // Seed AGENTS.md with the discovery marker and the full routing map.
        // This file is the source of truth for where content belongs — agents
        // that enter this ward read it on entry and follow it exactly.
        //
        // Re-seed on every startup IF the existing content starts with our
        // `<!-- obsidian-vault -->` marker (i.e. we wrote it previously, not
        // the user). This lets template updates flow through on gateway
        // restart without preserving a user-hand-edited file.
        let agents_md = wiki_dir.join("AGENTS.md");
        let should_seed = match std::fs::read_to_string(&agents_md) {
            Ok(existing) => existing.starts_with("<!-- obsidian-vault -->"),
            Err(_) => true, // missing → seed
        };
        if should_seed {
            let content = format!(
                "<!-- obsidian-vault -->\n\
                 # {wiki_name}\n\n\
                 ## Purpose / Scope\n\
                 Obsidian-style vault. Producer skills (book-reader, stock-analysis, news-research, …) emit vault-ready folders in their origin ward; the `wiki` skill promotes them here. **This AGENTS.md is the authoritative routing map.** If a memory fact contradicts it, this file wins.\n\n\
                 - **IN scope** — promoting producer-emitted, vault-ready folders from any origin ward into the numbered Obsidian tree; whole-folder copy; routing unmatched items to `00_Inbox/`.\n\
                 - **OUT of scope** — running code, research, or data fetching; rewriting promoted content; writing to user-managed folders; deleting from origin wards. Tasks needing any of these belong in another ward.\n\n\
                 ## Folder map — what goes where\n\n\
                 | Vault path | What lives here | Producer source |\n\
                 | --- | --- | --- |\n\
                 | `00_Inbox/` | Unclassified items awaiting manual sorting. Never delete; the user reviews periodically. | Anything that fails classification |\n\
                 | `10_Journal/Daily/` | One `YYYY-MM-DD.md` per day. | Journal skill (future) |\n\
                 | `10_Journal/Weekly/` | One `YYYY-Www.md` per ISO week. | Journal skill (future) |\n\
                 | `20_Projects/<project>/` | Agent-produced final project reports and deliverables. One folder per project. | `reports/<project>/` in origin ward |\n\
                 | `30_Library/Books/<slug>/` | A book as `_index.md` + `chunks/ch-NN.md` + `entities/<type>-<slug>.md`. `<slug>` is kebab-case from the title (strip leading articles). | `books/<slug>/` in origin ward (book-reader) |\n\
                 | `30_Library/Articles/<slug>/` | An article as `_index.md` (+ optional supporting files). `<slug>` is kebab-case from the title. | `articles/<slug>/` in origin ward (article-reader) |\n\
                 | `40_Research/<archetype>/<subject>/<date-slug>/` | Research snapshots. `<archetype>` is the producer skill name (`stock-analysis`, `news-research`, `product-research`, `competitive-analysis`, `academic-research`, `market-research`, `technical-research`, `policy-research`). `<subject>` is kebab-case. `<date-slug>` is ISO date with optional suffix. | `research/<archetype>/<subject>/<date-slug>/` |\n\
                 | `50_Resources/` | Durable reference material the user curates. | Manual only — `wiki` skill does not write here. |\n\
                 | `60_Archive/` | Superseded or retired content. Move here manually when an item is no longer current. | Manual only. |\n\
                 | `70_Assets/Knowledge_Graphs/` | KG exports (DB dumps) if generated by a separate tool. | Reserved — `wiki` does not write here. |\n\
                 | `70_Assets/Images/` | Loose images from any ward. Renamed `<ward>__<basename>` on copy to avoid collisions. | `**/*.{{png,jpg,jpeg,svg,gif,webp}}` in origin ward |\n\
                 | `70_Assets/Documents/` | Loose PDFs from any ward. Renamed `<ward>__<basename>` on copy. | `**/*.pdf` in origin ward |\n\
                 | `_zztemplates/` | Obsidian note templates the user maintains. | Manual only — the skill never writes or reads here. |\n\n\
                 ## Slug rules (the #1 failure mode)\n\n\
                 Folder names under `30_Library/Books/`, `30_Library/Articles/`, `40_Research/<archetype>/`, `20_Projects/` are always **kebab-case slugs**, never display titles:\n\n\
                 - `30_Library/Books/christmas-carol/` ✅  not `30_Library/Books/A Christmas Carol/` ❌\n\
                 - `30_Library/Books/pride-and-prejudice/` ✅  not `30_Library/Books/Pride and Prejudice/` ❌\n\
                 - `40_Research/stock-analysis/tsla/2026-04-16-q1/` ✅  not `40_Research/Stock Analysis/TSLA Q1 2026/` ❌\n\n\
                 The display title lives in `_index.md` frontmatter (`title:`) and in wikilink aliases (`[[slug|Display Title]]`). The filesystem always uses the slug.\n\n\
                 ## Routing contract for the wiki skill\n\n\
                 The skill performs **whole-folder copy** with absolute paths, no content rewriting. For each producer folder in the origin ward:\n\n\
                 1. Compute source path: `SRC=<origin-ward>/<producer-folder>` (e.g. `<origin>/books/christmas-carol/`).\n\
                 2. Compute destination path per the folder map above: `DEST=<wiki-ward>/<vault-path>/<slug>/`.\n\
                 3. Copy `cp -a \"$SRC\" \"$DEST\"`. Preserve timestamps; preserve names; preserve nested structure.\n\
                 4. If the source doesn't match any rule, route to `00_Inbox/<relative-path>` — do NOT guess a category.\n\n\
                 ## Hard don'ts\n\n\
                 - Do NOT invent folders outside the numbered tree (`Literature/`, `StockResearch/`, `Books/`, etc. are WRONG — use the numbered paths).\n\
                 - Do NOT use display-case folder names with spaces or capitals.\n\
                 - Do NOT rewrite frontmatter, wikilinks, or markdown during the copy — producer skills own the content shape.\n\
                 - Do NOT delete from the origin ward.\n\
                 - Do NOT write into `50_Resources/`, `60_Archive/`, `_zztemplates/`, or `70_Assets/Knowledge_Graphs/` — those are user-managed or reserved.\n\
                 - Do NOT run code, fetch data, or do research in this ward. It is content-only.\n\
                 - Do NOT edit promoted files outside their `<!-- manual -->` blocks — the skill overwrites on re-promotion.\n\n\
                 ## Handoff\n\n\
                 On completion, return a JSON object summarizing the promotion run: `{{ \"status\": \"ok | partial | failed\", \"summary\": \"one line\", \"promoted\": [\"<vault-path>\"], \"inboxed\": [\"<vault-path>\"], \"skipped\": [\"<path>\"] }}`.\n\n\
                 `promoted` = folders copied to a numbered path; `inboxed` = folders routed to `00_Inbox/` because no rule matched; `skipped` = paths intentionally left in the origin ward.\n\n\
                 ## Discovery marker\n\n\
                 The first line of this file (`<!-- obsidian-vault -->`) is the marker the wiki skill uses to find this ward via `ward(action=\"list\")`. Do not remove it.\n"
            );
            let _ = std::fs::write(&agents_md, content);
        }

        // Seed memory-bank/ scaffold so the ward matches the standard shape.
        let memory_bank = wiki_dir.join("memory-bank");
        let _ = std::fs::create_dir_all(&memory_bank);
        for file in ["ward.md", "structure.md", "core_docs.md"] {
            let path = memory_bank.join(file);
            if !path.exists() {
                let _ = std::fs::write(&path, "");
            }
        }

        // Mark the bundled provenance so the curator never archives the wiki.
        if let Err(e) = WardUsage::new(wards_dir).mark_created(wiki_name, WardProvenance::Bundled) {
            tracing::warn!(
                ward = %wiki_name,
                error = %e,
                "ward_usage: failed to mark wiki as bundled"
            );
        }

        tracing::info!("Wiki vault ward ready at {}", wiki_dir.display());
    }
}

#[derive(Clone, Copy, Debug)]
struct LocalProviderStatus {
    memory_store: bool,
    kg_store: bool,
    ingestion_queue: bool,
    compaction_store: bool,
    belief_store: bool,
}

fn append_actor_visible_capabilities(
    catalog: &mut ContextCapabilityCatalog,
    capabilities: Vec<ContextCapability>,
) {
    let mut seen: BTreeSet<String> = catalog
        .capabilities
        .iter()
        .map(|capability| capability.id.clone())
        .collect();

    for capability in capabilities {
        if !capability.actor_policy.contains(&catalog.actor_kind) {
            continue;
        }
        if seen.insert(capability.id.clone()) {
            catalog.capabilities.push(capability);
        }
    }
}

fn local_context_provider_capabilities(status: LocalProviderStatus) -> Vec<ContextCapability> {
    let read_policy = all_actor_policy();
    let mut capabilities = Vec::new();

    capabilities.push(ContextCapability {
        id: "memory:facts".to_string(),
        kind: ContextCapabilityKind::Resource,
        display_name: "Memory Facts".to_string(),
        description: "Semantic memory fact resource exposed through the selected memory provider."
            .to_string(),
        actor_policy: read_policy.clone(),
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            }
        })),
        output_schema: None,
        resource_uri_template: Some("zbot://memory/facts{?query,limit}".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(500),
        health: health_for(status.memory_store),
        owner_crate: Some("zbot-stores-traits".to_string()),
        audit_policy: Some("memory_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("context:memory_recall".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "memory:recall_unified".to_string(),
        kind: ContextCapabilityKind::ContextGraph,
        display_name: "Unified Recall".to_string(),
        description: "Context packet provider for semantic memory, procedures, wiki, and graph-enriched recall."
            .to_string(),
        actor_policy: read_policy.clone(),
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string" },
                "session_id": { "type": "string" },
                "agent_id": { "type": "string" }
            },
            "required": ["prompt"]
        })),
        output_schema: None,
        resource_uri_template: Some("zbot://context/recall/unified{?prompt,session_id,agent_id}".to_string()),
        cost_hint: Some(ContextCostHint::Moderate),
        latency_hint: Some(ContextLatencyHint::Slow),
        token_hint: Some(1_500),
        health: health_for(status.memory_store),
        owner_crate: Some("gateway-memory".to_string()),
        audit_policy: Some("context_packet_trace".to_string()),
        default_visible: false,
        visibility_policy: "context_provider_only".to_string(),
        split_target: Some("context_packet:recall_unified".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "knowledge_graph:entities".to_string(),
        kind: ContextCapabilityKind::ContextGraph,
        display_name: "Knowledge Graph Entities".to_string(),
        description: "Entity and relationship graph resource exposed through the selected knowledge provider."
            .to_string(),
        actor_policy: read_policy.clone(),
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500 }
            }
        })),
        output_schema: None,
        resource_uri_template: Some("zbot://knowledge-graph/entities{?query,limit}".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(900),
        health: health_for(status.kg_store),
        owner_crate: Some("zbot-stores-traits".to_string()),
        audit_policy: Some("graph_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("context_graph:knowledge_graph".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "knowledge_graph:ingestion_queue".to_string(),
        kind: ContextCapabilityKind::Catalog,
        display_name: "Knowledge Graph Ingestion Queue".to_string(),
        description: "Ingestion queue health and intake boundary for evidence destined for the knowledge graph."
            .to_string(),
        actor_policy: vec![ContextActorKind::Root, ContextActorKind::WardAgent],
        risk_level: ContextRiskLevel::Moderate,
        side_effects: ContextSideEffects::WriteLocal,
        input_schema: None,
        output_schema: None,
        resource_uri_template: Some("zbot://knowledge-graph/ingestion-queue".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Background),
        token_hint: Some(150),
        health: health_for(status.ingestion_queue),
        owner_crate: Some("gateway-execution".to_string()),
        audit_policy: Some("evidence_intake_audit".to_string()),
        default_visible: false,
        visibility_policy: "catalog_only".to_string(),
        split_target: Some("tool:ingest".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "memory:compaction".to_string(),
        kind: ContextCapabilityKind::Catalog,
        display_name: "Memory Compaction".to_string(),
        description:
            "Compaction audit resource for sleep-time memory maintenance and consolidation."
                .to_string(),
        actor_policy: vec![ContextActorKind::Root, ContextActorKind::WardAgent],
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: None,
        output_schema: None,
        resource_uri_template: Some("zbot://memory/compaction".to_string()),
        cost_hint: Some(ContextCostHint::Free),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(200),
        health: health_for(status.compaction_store),
        owner_crate: Some("gateway-memory".to_string()),
        audit_policy: Some("compaction_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "catalog_only".to_string(),
        split_target: Some("context:memory_compaction".to_string()),
    });

    capabilities.push(ContextCapability {
        id: "memory:belief_network".to_string(),
        kind: ContextCapabilityKind::ContextGraph,
        display_name: "Belief Network".to_string(),
        description: "Belief and contradiction graph resource when the belief network is enabled."
            .to_string(),
        actor_policy: read_policy,
        risk_level: ContextRiskLevel::Low,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: None,
        output_schema: None,
        resource_uri_template: Some("zbot://memory/belief-network".to_string()),
        cost_hint: Some(ContextCostHint::Cheap),
        latency_hint: Some(ContextLatencyHint::Fast),
        token_hint: Some(700),
        health: health_for(status.belief_store),
        owner_crate: Some("gateway-memory".to_string()),
        audit_policy: Some("belief_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("context_graph:belief_network".to_string()),
    });

    capabilities
}

fn mcp_catalog_capabilities(
    summaries: Vec<gateway_services::mcp::McpServerSummary>,
) -> Vec<ContextCapability> {
    summaries
        .into_iter()
        .map(|summary| {
            let id_component = catalog_id_component(&summary.id);
            let mut description = summary.description;
            if let Some(auth_status) = summary.auth_status {
                if !auth_status.is_empty() {
                    description = format!("{description} Auth status: {auth_status}.");
                }
            }

            ContextCapability {
                id: format!("mcp:{id_component}"),
                kind: ContextCapabilityKind::Catalog,
                display_name: format!("MCP: {}", summary.name),
                description,
                actor_policy: all_actor_policy(),
                risk_level: ContextRiskLevel::Low,
                side_effects: ContextSideEffects::None,
                input_schema: None,
                output_schema: None,
                resource_uri_template: Some(format!("zbot://mcp/{id_component}")),
                cost_hint: Some(ContextCostHint::Free),
                latency_hint: Some(ContextLatencyHint::Local),
                token_hint: Some(120),
                health: if summary.enabled {
                    ContextCapabilityHealth::Available
                } else {
                    ContextCapabilityHealth::Disabled
                },
                owner_crate: Some("gateway-services".to_string()),
                audit_policy: Some("catalog_read".to_string()),
                default_visible: false,
                visibility_policy: "catalog_only".to_string(),
                split_target: Some(format!("mcp_transport:{}", summary.transport_type)),
            }
        })
        .collect()
}

fn connector_catalog_capabilities(connectors: Vec<ConnectorInfo>) -> Vec<ContextCapability> {
    let mut capabilities = Vec::new();

    for connector in connectors {
        let connector_id = catalog_id_component(&connector.id);
        let connector_name = connector.name;
        for resource in connector.resources {
            capabilities.push(connector_resource_capability(
                &connector_name,
                &connector_id,
                resource,
            ));
        }
        for capability in connector.capabilities {
            capabilities.push(connector_action_capability(
                &connector_name,
                &connector_id,
                capability,
            ));
        }
    }

    capabilities
}

fn connector_resource_capability(
    connector_name: &str,
    connector_id: &str,
    resource: ResourceInfo,
) -> ContextCapability {
    let resource_id = catalog_id_component(&resource.name);
    let method = resource.method.to_uppercase();
    let description = resource.description.unwrap_or_else(|| {
        format!(
            "Read {} from connector {} via {}.",
            resource.name, connector_name, method
        )
    });

    ContextCapability {
        id: format!("connector:{connector_id}:resource:{resource_id}"),
        kind: ContextCapabilityKind::Resource,
        display_name: format!("{}: {}", connector_name, resource.name),
        description,
        actor_policy: connector_actor_policy(),
        risk_level: ContextRiskLevel::Moderate,
        side_effects: ContextSideEffects::ReadExternal,
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "params": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            }
        })),
        output_schema: None,
        resource_uri_template: Some(format!(
            "zbot://connectors/{connector_id}/resources/{resource_id}{{?params}}"
        )),
        cost_hint: Some(ContextCostHint::Moderate),
        latency_hint: Some(ContextLatencyHint::Slow),
        token_hint: Some(800),
        health: ContextCapabilityHealth::Available,
        owner_crate: Some("gateway-connectors".to_string()),
        audit_policy: Some("connector_resource_read_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("tool:query_resource?action=query".to_string()),
    }
}

fn connector_action_capability(
    connector_name: &str,
    connector_id: &str,
    capability: CapabilityInfo,
) -> ContextCapability {
    let capability_id = catalog_id_component(&capability.name);
    let schema = if capability.schema.is_null() {
        None
    } else {
        Some(capability.schema)
    };

    ContextCapability {
        id: format!("connector:{connector_id}:capability:{capability_id}"),
        kind: ContextCapabilityKind::Tool,
        display_name: format!("{}: {}", connector_name, capability.name),
        description: capability.description.unwrap_or_else(|| {
            format!(
                "Invoke connector capability {} on {}.",
                capability.name, connector_name
            )
        }),
        actor_policy: connector_actor_policy(),
        risk_level: ContextRiskLevel::Moderate,
        side_effects: ContextSideEffects::WriteExternal,
        input_schema: schema,
        output_schema: None,
        resource_uri_template: Some(format!(
            "zbot://connectors/{connector_id}/capabilities/{capability_id}"
        )),
        cost_hint: Some(ContextCostHint::Moderate),
        latency_hint: Some(ContextLatencyHint::Slow),
        token_hint: Some(900),
        health: ContextCapabilityHealth::Available,
        owner_crate: Some("gateway-connectors".to_string()),
        audit_policy: Some("connector_capability_invoke_audit".to_string()),
        default_visible: false,
        visibility_policy: "resource_catalog_only".to_string(),
        split_target: Some("tool:query_resource?action=invoke".to_string()),
    }
}

fn all_actor_policy() -> Vec<ContextActorKind> {
    vec![
        ContextActorKind::Root,
        ContextActorKind::DelegatedExecutor,
        ContextActorKind::DelegatedReviewer,
        ContextActorKind::WardAgent,
    ]
}

fn connector_actor_policy() -> Vec<ContextActorKind> {
    vec![ContextActorKind::Root, ContextActorKind::WardAgent]
}

fn health_for(enabled: bool) -> ContextCapabilityHealth {
    if enabled {
        ContextCapabilityHealth::Available
    } else {
        ContextCapabilityHealth::Disabled
    }
}

fn catalog_id_component(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut last_was_separator = false;
    for ch in raw.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(ch.to_ascii_lowercase())
        } else if !last_was_separator {
            last_was_separator = true;
            Some('_')
        } else {
            None
        };
        if let Some(ch) = next {
            normalized.push(ch);
        }
    }
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "unnamed".to_string()
    } else {
        normalized.to_string()
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
    fn local_context_provider_catalog_entries_report_resource_health() {
        let capabilities = local_context_provider_capabilities(LocalProviderStatus {
            memory_store: true,
            kg_store: true,
            ingestion_queue: false,
            compaction_store: false,
            belief_store: false,
        });

        let memory = capabilities
            .iter()
            .find(|capability| capability.id == "memory:facts")
            .expect("memory resource");
        assert_eq!(memory.kind, ContextCapabilityKind::Resource);
        assert_eq!(memory.health, ContextCapabilityHealth::Available);
        assert!(!memory.default_visible);
        assert!(memory
            .actor_policy
            .contains(&ContextActorKind::DelegatedReviewer));

        let graph = capabilities
            .iter()
            .find(|capability| capability.id == "knowledge_graph:entities")
            .expect("knowledge graph resource");
        assert_eq!(graph.kind, ContextCapabilityKind::ContextGraph);
        assert_eq!(
            graph.resource_uri_template.as_deref(),
            Some("zbot://knowledge-graph/entities{?query,limit}")
        );

        let ingestion = capabilities
            .iter()
            .find(|capability| capability.id == "knowledge_graph:ingestion_queue")
            .expect("ingestion queue resource");
        assert_eq!(ingestion.health, ContextCapabilityHealth::Disabled);
        assert_eq!(ingestion.side_effects, ContextSideEffects::WriteLocal);
    }

    #[test]
    fn connector_catalog_entries_use_logical_resource_uris() {
        let connectors = vec![ConnectorInfo {
            id: "Local Mail".to_string(),
            name: "Local Mail".to_string(),
            resources: vec![ResourceInfo {
                name: "Recent Messages".to_string(),
                uri: "http://127.0.0.1:9999/private/messages".to_string(),
                method: "GET".to_string(),
                description: Some("Read recent messages.".to_string()),
            }],
            capabilities: vec![CapabilityInfo {
                name: "Send Message".to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "to": { "type": "string" },
                        "body": { "type": "string" }
                    }
                }),
                description: None,
            }],
        }];

        let capabilities = connector_catalog_capabilities(connectors);
        let resource = capabilities
            .iter()
            .find(|capability| capability.id == "connector:local_mail:resource:recent_messages")
            .expect("connector resource");
        assert_eq!(resource.kind, ContextCapabilityKind::Resource);
        assert_eq!(resource.side_effects, ContextSideEffects::ReadExternal);
        assert_eq!(
            resource.resource_uri_template.as_deref(),
            Some("zbot://connectors/local_mail/resources/recent_messages{?params}")
        );
        assert!(!serde_json::to_string(resource)
            .unwrap()
            .contains("127.0.0.1"));

        let action = capabilities
            .iter()
            .find(|capability| capability.id == "connector:local_mail:capability:send_message")
            .expect("connector action");
        assert_eq!(action.side_effects, ContextSideEffects::WriteExternal);
        assert_eq!(
            action.actor_policy,
            vec![ContextActorKind::Root, ContextActorKind::WardAgent]
        );
    }

    #[test]
    fn resource_catalog_filters_entries_by_actor_and_dedupes() {
        let mut catalog = ContextCapabilityCatalog {
            version: "test".to_string(),
            actor_kind: ContextActorKind::DelegatedReviewer,
            session_id: None,
            agent_id: None,
            capabilities: vec![ContextCapability {
                id: "memory:facts".to_string(),
                kind: ContextCapabilityKind::Tool,
                display_name: "Existing".to_string(),
                description: "Existing entry wins.".to_string(),
                actor_policy: vec![ContextActorKind::DelegatedReviewer],
                risk_level: ContextRiskLevel::Low,
                side_effects: ContextSideEffects::None,
                input_schema: None,
                output_schema: None,
                resource_uri_template: None,
                cost_hint: None,
                latency_hint: None,
                token_hint: None,
                health: ContextCapabilityHealth::Available,
                owner_crate: None,
                audit_policy: None,
                default_visible: true,
                visibility_policy: "default_visible".to_string(),
                split_target: None,
            }],
        };

        let mut extra = local_context_provider_capabilities(LocalProviderStatus {
            memory_store: true,
            kg_store: true,
            ingestion_queue: true,
            compaction_store: true,
            belief_store: true,
        });
        extra.extend(connector_catalog_capabilities(vec![ConnectorInfo {
            id: "crm".to_string(),
            name: "CRM".to_string(),
            resources: vec![ResourceInfo {
                name: "contacts".to_string(),
                uri: "https://crm.example/resources/contacts".to_string(),
                method: "GET".to_string(),
                description: None,
            }],
            capabilities: vec![],
        }]));

        append_actor_visible_capabilities(&mut catalog, extra);
        let ids: BTreeSet<_> = catalog
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect();

        assert_eq!(
            catalog
                .capabilities
                .iter()
                .filter(|capability| capability.id == "memory:facts")
                .count(),
            1
        );
        assert!(ids.contains("knowledge_graph:entities"));
        assert!(!ids.contains("knowledge_graph:ingestion_queue"));
        assert!(!ids.contains("connector:crm:resource:contacts"));
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
    fn ensure_wards_dir_creates_scratch_and_wiki_subtrees() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();

        assert!(state.vault_dir.join("wards").join("scratch").is_dir());

        let wiki = state.vault_dir.join("wards").join("wiki");
        assert!(wiki.is_dir());
        for folder in [
            "00_Inbox",
            "20_Projects",
            "30_Library/Books",
            "40_Research",
            "50_Resources",
            "60_Archive",
            "70_Assets/Images",
            "_zztemplates",
        ] {
            assert!(wiki.join(folder).is_dir());
        }

        let agents_md = std::fs::read_to_string(wiki.join("AGENTS.md")).expect("agents.md");
        assert!(agents_md.starts_with("<!-- obsidian-vault -->"));

        for f in ["ward.md", "structure.md", "core_docs.md"] {
            assert!(wiki.join("memory-bank").join(f).exists());
        }
    }

    #[test]
    fn ensure_wards_dir_is_idempotent_and_preserves_user_edits() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();
        let agents_md_path = state.vault_dir.join("wards").join("wiki").join("AGENTS.md");

        std::fs::write(&agents_md_path, "user-authored content").unwrap();
        state.ensure_wards_dir();
        let after = std::fs::read_to_string(&agents_md_path).unwrap();
        assert_eq!(after, "user-authored content");
    }

    #[test]
    fn ensure_wards_dir_reseeds_when_marker_present() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();
        let agents_md_path = state.vault_dir.join("wards").join("wiki").join("AGENTS.md");

        std::fs::write(
            &agents_md_path,
            "<!-- obsidian-vault -->\nold seed content\n",
        )
        .unwrap();
        state.ensure_wards_dir();
        let after = std::fs::read_to_string(&agents_md_path).unwrap();
        assert!(after.starts_with("<!-- obsidian-vault -->"));
        assert!(after.contains("Folder map"));
    }

    #[test]
    fn ensure_wiki_ward_handles_custom_name() {
        let (_dir, state) = make_temp_state();
        let wards = state.vault_dir.join("wards");
        std::fs::create_dir_all(&wards).unwrap();
        state.ensure_wiki_ward(&wards, "knowledge");

        let custom = wards.join("knowledge");
        assert!(custom.is_dir());
        assert!(custom.join("AGENTS.md").exists());
        let content = std::fs::read_to_string(custom.join("AGENTS.md")).unwrap();
        assert!(content.contains("# knowledge"));
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

        assert!(state.vault_dir.join("wards").join("scratch").is_dir());
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
