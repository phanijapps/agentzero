//! # Application State
//!
//! Shared state for the gateway application.

mod bootstrap;
mod capability_catalog;
pub mod groups;

pub(crate) use bootstrap::FlatAppState;
pub(crate) mod persistence_factory;
mod seeded_defaults;
mod seeding;

use crate::connectors::ConnectorRegistry;
use crate::cron::CronScheduler;
use crate::events::EventBus;
use crate::execution::{DelegationRegistry, SessionArchiver};
use crate::hooks::HookRegistry;
use crate::services::{
    AgentService, McpService, ModelRegistry, ProviderService, RuntimeService, SettingsService,
    SharedVaultPaths, SkillService,
};

use api_logs::LogService;
use execution_state::{StateService, WorkStore};
use gateway_services::EmbeddingService;

use std::sync::Arc;
use zbot_engram_adapter::GovernanceCapabilityHealth;
use zbot_runtime_sqlite::{DatabaseManager, DistillationRepository};

/// Shared application state for the gateway.
#[derive(Clone)]
pub struct AppState {
    // --- W2 state groups (see state/groups.rs) ---------------------------
    // The single source of truth. Legacy flat access lives on as
    // name-identical accessor methods (below) so call sites stay
    // mechanical: `state.memory_store` -> `state.memory_store()`.
    pub stores: groups::StoresState,
    pub services: groups::ServicesState,
    pub execution: groups::ExecutionState,
    pub transport: groups::TransportState,
    pub workers: groups::WorkersState,
    pub vault: groups::VaultState,
}
/// The flat construction shape of [`AppState`] (W2): the 49 fields
/// every builder already produces; `from_flat` clones the group Arcs
/// and moves the rest. Private — the group fields are the public future.
impl AppState {
    /// The single construction path (W2): take the flat fields, clone the
    /// Arcs the groups need, then move the rest into `AppState`.
    fn from_flat(flat: FlatAppState) -> Self {
        let FlatAppState {
            agents,
            skills,
            provider_service,
            mcp_service,
            runtime,
            event_bus,
            hook_registry,
            delegation_registry,
            messages,
            session_meta,
            checkpoints,
            autonomy,
            slim_logs,
            trace_analytics,
            settings,
            log_service,
            state_service,
            durable_work_store,
            durable_work_transport,
            connector_registry,
            bridge_registry,
            bridge_outbox,
            bridge_bus,
            memory_store,
            goal_store,
            distillation_repo,
            distiller,
            episode_store,
            wiki_store,
            procedure_store,
            kg_episode_store,
            kg_store,
            governance_health,
            ingestion_queue,
            ingestion_backpressure,
            cron_scheduler,
            plugin_manager,
            session_archiver,
            sleep_time_worker,
            compaction_store,
            belief_store,
            belief_contradiction_store,
            belief_network_activity,
            model_registry,
            embedding_service,
            paths,
            vault_dir,
            advertiser,
            advertise_handle,
        } = flat;
        Self {
            stores: groups::StoresState {
                memory_store,
                goal_store,
                distillation_repo,
                episode_store,
                wiki_store,
                procedure_store,
                kg_episode_store,
                kg_store,
                governance_health,
                compaction_store,
                belief_store,
                belief_contradiction_store,
                belief_network_activity,
                messages,
                session_meta,
                checkpoints,
                slim_logs,
            },
            services: groups::ServicesState {
                agents,
                skills,
                provider_service,
                mcp_service,
                settings,
                log_service,
                state_service,
                model_registry,
                embedding_service,
            },
            execution: groups::ExecutionState {
                runtime,
                event_bus,
                hook_registry,
                delegation_registry,
                autonomy,
                trace_analytics,
                ingestion_queue,
                ingestion_backpressure,
            },
            transport: groups::TransportState {
                durable_work_transport,
                connector_registry,
                bridge_registry,
                bridge_outbox,
                bridge_bus,
                plugin_manager,
                advertiser,
                advertise_handle,
            },
            workers: groups::WorkersState {
                durable_work_store,
                distiller,
                cron_scheduler,
                session_archiver,
                sleep_time_worker,
            },
            vault: groups::VaultState { paths, vault_dir },
        }
    }
}

// ===========================================================================
// Legacy flat-surface accessors (W3)
//
// The 49 former public fields now read through the six groups. Method
// names match the old field names exactly, so the call-site migration is
// purely mechanical: `state.memory_store` -> `state.memory_store()`.
// New code should depend on the group it needs (`state.stores`, ...).
// ===========================================================================
impl AppState {
    // --- group accessors (W4) ----------------------------------------------
    /// The stores group — declare this instead of the whole AppState when a
    /// consumer needs 2+ store handles.
    pub fn stores(&self) -> &groups::StoresState {
        &self.stores
    }

    /// The services group — capability/config services.
    pub fn services(&self) -> &groups::ServicesState {
        &self.services
    }

    /// The execution group — engine handles (runtime, hooks, events, ingestion).
    pub fn execution(&self) -> &groups::ExecutionState {
        &self.execution
    }

    /// The transport group — gateway in/out edges (bridges, connectors, mDNS).
    pub fn transport(&self) -> &groups::TransportState {
        &self.transport
    }

    /// The workers group — background loops (sleep, archiver, cron, distiller).
    pub fn workers(&self) -> &groups::WorkersState {
        &self.workers
    }

    /// The vault group — environment (paths).
    pub fn vault(&self) -> &groups::VaultState {
        &self.vault
    }

    // --- stores -------------------------------------------------------------
    /// Memory-fact store — the single read/write surface for memory facts.
    pub fn memory_store(&self) -> Option<Arc<dyn zbot_stores_traits::MemoryFactStore>> {
        self.stores.memory_store.clone()
    }
    /// Backend-neutral active goals (intent boost, goal tool).
    pub fn goal_store(&self) -> Option<Arc<dyn zbot_stores_traits::GoalStore>> {
        self.stores.goal_store.clone()
    }
    /// Distillation run bookkeeping repository.
    pub fn distillation_repo(&self) -> Option<Arc<DistillationRepository>> {
        self.stores.distillation_repo.clone()
    }
    /// Session episode store.
    pub fn episode_store(&self) -> Option<Arc<dyn zbot_stores_traits::EpisodeStore>> {
        self.stores.episode_store.clone()
    }
    /// Wiki store.
    pub fn wiki_store(&self) -> Option<Arc<dyn zbot_stores_traits::WikiStore>> {
        self.stores.wiki_store.clone()
    }
    /// Procedure store.
    pub fn procedure_store(&self) -> Option<Arc<dyn zbot_stores_traits::ProcedureStore>> {
        self.stores.procedure_store.clone()
    }
    /// kg-ingestion-episode store.
    pub fn kg_episode_store(&self) -> Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>> {
        self.stores.kg_episode_store.clone()
    }
    /// Knowledge-graph store.
    pub fn kg_store(&self) -> Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>> {
        self.stores.kg_store.clone()
    }
    /// Governance health snapshot for Observatory routes.
    pub fn governance_health(&self) -> Option<GovernanceCapabilityHealth> {
        self.stores.governance_health.clone()
    }
    /// Compaction audit store.
    pub fn compaction_store(&self) -> Option<Arc<dyn zbot_stores_traits::CompactionStore>> {
        self.stores.compaction_store.clone()
    }
    /// Belief store (Belief Network opt-in).
    pub fn belief_store(&self) -> Option<Arc<dyn zbot_stores_traits::BeliefStore>> {
        self.stores.belief_store.clone()
    }
    /// Belief-contradiction store (Belief Network opt-in).
    pub fn belief_contradiction_store(
        &self,
    ) -> Option<Arc<dyn zbot_stores_traits::BeliefContradictionStore>> {
        self.stores.belief_contradiction_store.clone()
    }
    /// Recent Belief Network worker activity recorder.
    pub fn belief_network_activity(
        &self,
    ) -> Option<Arc<gateway_memory::RecentBeliefNetworkActivity>> {
        self.stores.belief_network_activity.clone()
    }
    /// Append-only conversation log.
    pub fn messages(&self) -> Arc<dyn zbot_conversation::MessageStore> {
        self.stores.messages.clone()
    }
    /// Narrow session metadata reads.
    pub fn session_meta(&self) -> Arc<dyn zbot_conversation::SessionMetaStore> {
        self.stores.session_meta.clone()
    }
    /// Versioned agent-state checkpoints.
    pub fn checkpoints(&self) -> Arc<dyn zbot_conversation::CheckpointStore> {
        self.stores.checkpoints.clone()
    }
    /// Slim `execution_logs` (the `/api/logs` UI source).
    pub fn slim_logs(&self) -> Arc<dyn zbot_trace::SlimLogStore> {
        self.stores.slim_logs.clone()
    }

    // --- services -----------------------------------------------------------
    /// Agent configuration service.
    pub fn agents(&self) -> Arc<AgentService> {
        self.services.agents.clone()
    }
    /// Skill service.
    pub fn skills(&self) -> Arc<SkillService> {
        self.services.skills.clone()
    }
    /// LLM provider service.
    pub fn provider_service(&self) -> Arc<ProviderService> {
        self.services.provider_service.clone()
    }
    /// MCP service.
    pub fn mcp_service(&self) -> Arc<McpService> {
        self.services.mcp_service.clone()
    }
    /// Settings service.
    pub fn settings(&self) -> Arc<SettingsService> {
        self.services.settings.clone()
    }
    /// Execution log service.
    pub fn log_service(&self) -> Arc<LogService<DatabaseManager>> {
        self.services.log_service.clone()
    }
    /// Execution state service.
    pub fn state_service(&self) -> Arc<StateService<DatabaseManager>> {
        self.services.state_service.clone()
    }
    /// Fallback model metadata registry.
    pub fn model_registry(&self) -> Arc<ModelRegistry> {
        self.services.model_registry.clone()
    }
    /// Embedding service (live client, backend swap).
    pub fn embedding_service(&self) -> Arc<EmbeddingService> {
        self.services.embedding_service.clone()
    }

    // --- execution ----------------------------------------------------------
    /// Runtime service for agent execution.
    pub fn runtime(&self) -> Arc<RuntimeService> {
        self.execution.runtime.clone()
    }
    /// Event bus for broadcasting events.
    pub fn event_bus(&self) -> Arc<EventBus> {
        self.execution.event_bus.clone()
    }
    /// Inbound trigger hook registry.
    pub fn hook_registry(&self) -> Option<Arc<HookRegistry>> {
        self.execution.hook_registry.clone()
    }
    /// Delegation registry.
    pub fn delegation_registry(&self) -> Arc<DelegationRegistry> {
        self.execution.delegation_registry.clone()
    }
    /// Durable autonomy threads.
    pub fn autonomy(&self) -> Arc<dyn zbot_conversation::AutonomyStore> {
        self.execution.autonomy.clone()
    }
    /// Cross-session trace analytics.
    pub fn trace_analytics(&self) -> Arc<zbot_trace::TraceAnalytics> {
        self.execution.trace_analytics.clone()
    }
    /// Streaming ingestion queue.
    pub fn ingestion_queue(&self) -> Option<Arc<gateway_execution::ingest::IngestionQueue>> {
        self.execution.ingestion_queue.clone()
    }
    /// Ingestion backpressure gate.
    pub fn ingestion_backpressure(&self) -> Option<Arc<gateway_execution::ingest::Backpressure>> {
        self.execution.ingestion_backpressure.clone()
    }

    // --- transport ----------------------------------------------------------
    /// Durable work wake path.
    pub fn durable_work_transport(&self) -> Arc<gateway_bus::LocalWorkTransport> {
        self.transport.durable_work_transport.clone()
    }
    /// External connector registry.
    pub fn connector_registry(&self) -> Arc<ConnectorRegistry> {
        self.transport.connector_registry.clone()
    }
    /// Worker WebSocket bridge registry.
    pub fn bridge_registry(&self) -> Arc<gateway_bridge::BridgeRegistry> {
        self.transport.bridge_registry.clone()
    }
    /// Bridge outbox.
    pub fn bridge_outbox(&self) -> Arc<gateway_bridge::OutboxRepository> {
        self.transport.bridge_outbox.clone()
    }
    /// Gateway bus (set during server start).
    pub fn bridge_bus(&self) -> Option<Arc<dyn gateway_bus::GatewayBus>> {
        self.transport.bridge_bus.clone()
    }
    /// STDIO plugin lifecycle manager.
    pub fn plugin_manager(&self) -> Arc<gateway_bridge::PluginManager> {
        self.transport.plugin_manager.clone()
    }
    /// LAN service advertiser.
    pub fn advertiser(&self) -> Arc<dyn discovery::Advertiser> {
        self.transport.advertiser.clone()
    }
    /// Active mDNS advertise handle.
    pub fn advertise_handle(&self) -> Arc<std::sync::Mutex<Option<discovery::AdvertiseHandle>>> {
        self.transport.advertise_handle.clone()
    }

    // --- workers ------------------------------------------------------------
    /// Durable executable-work store.
    pub fn durable_work_store(&self) -> Arc<dyn WorkStore> {
        self.workers.durable_work_store.clone()
    }
    /// Session distiller.
    pub fn distiller(&self) -> Option<Arc<distillation::SessionDistiller>> {
        self.workers.distiller.clone()
    }
    /// Cron scheduler.
    pub fn cron_scheduler(&self) -> Option<Arc<CronScheduler>> {
        self.workers.cron_scheduler.clone()
    }
    /// Session archiver.
    pub fn session_archiver(&self) -> Option<Arc<SessionArchiver>> {
        self.workers.session_archiver.clone()
    }
    /// Sleep-time worker.
    pub fn sleep_time_worker(&self) -> Option<Arc<gateway_memory::sleep::SleepTimeWorker>> {
        self.workers.sleep_time_worker.clone()
    }

    // --- vault --------------------------------------------------------------
    /// Vault paths.
    pub fn paths(&self) -> SharedVaultPaths {
        self.vault.paths.clone()
    }
    /// Vault root path.
    pub fn vault_dir(&self) -> std::path::PathBuf {
        self.vault.vault_dir.clone()
    }
}

impl AppState {
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
        let tool_settings = self.settings().get_tool_settings().unwrap_or_default();

        if let Some(runner) = self.runtime().runner() {
            return runner.context_capability_catalog(
                actor_kind,
                tool_settings,
                session_id,
                agent_id,
            );
        }

        capability_catalog::ToolCatalog {
            paths: self.paths().clone(),
            state_service: self.state_service().clone(),
            messages: self.messages().clone(),
            model_registry: self.model_registry().clone(),
            memory_store: self.memory_store().clone(),
            kg_store: self.kg_store().clone(),
            ingestion_queue: self.ingestion_queue().clone(),
            kg_episode_store: self.kg_episode_store().clone(),
            goal_store: self.goal_store().clone(),
            procedure_store: self.procedure_store().clone(),
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
                memory_store: self.memory_store().is_some(),
                kg_store: self.kg_store().is_some(),
                ingestion_queue: self.ingestion_queue().is_some(),
                compaction_store: self.compaction_store().is_some(),
                belief_store: self.belief_store().is_some(),
            },
            mcp_service: self.mcp_service().clone(),
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
                self.connector_registry().clone(),
            )));
        let bridge_provider: Option<Arc<dyn agent_primitives::ConnectorResourceProvider>> =
            Some(Arc::new(gateway_bridge::BridgeResourceProvider::new(
                self.bridge_registry().clone(),
                self.bridge_outbox().clone(),
            )));

        Some(Arc::new(gateway_execution::CompositeResourceProvider::new(
            http_provider,
            bridge_provider,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::ConnectorService;
    use crate::events::EventBus;
    use crate::hooks::HookRegistry;
    use crate::services::VaultPaths;
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
        assert!(state.hook_registry().is_none());
        assert!(state.cron_scheduler().is_none());
        assert!(state.session_archiver().is_none());
        assert!(state.bridge_bus().is_none());
        assert!(state.memory_store().is_some());
        assert!(state.episode_store().is_some());
        assert!(state.wiki_store().is_some());
        assert!(state.procedure_store().is_some());
        assert!(state.kg_store().is_some());
        assert!(state.kg_episode_store().is_some());
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
        assert!(state.hook_registry().is_some());
    }

    #[test]
    fn ensure_wards_dir_creates_only_plain_root_catalog() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();
        let wards = state.vault_dir().join("wards");
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
        let index = state.vault_dir().join("wards").join("index.md");

        std::fs::write(&index, "# My catalog\n").unwrap();
        state.ensure_wards_dir();
        assert_eq!(std::fs::read_to_string(index).unwrap(), "# My catalog\n");
    }

    #[test]
    fn ensure_wards_dir_does_not_seed_legacy_wards() {
        let (_dir, state) = make_temp_state();
        state.ensure_wards_dir();
        state.ensure_wards_dir();
        assert!(!state.vault_dir().join("wards/scratch").exists());
        assert!(!state.vault_dir().join("wards/wiki").exists());
    }

    #[test]
    fn seed_default_skills_is_no_op_when_skills_dir_has_content() {
        let (_dir, state) = make_temp_state();
        let skills_dir = state.paths().vault_dir().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("sentinel.md"), "user").unwrap();

        state.seed_default_skills();
        assert!(skills_dir.join("sentinel.md").exists());
    }

    #[test]
    fn seed_default_skills_populates_empty_dir_from_templates() {
        let (_dir, state) = make_temp_state();
        let skills_dir = state.paths().vault_dir().join("skills");
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

        let registry_path = state.paths().seeded_defaults();
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

        assert!(state.vault_dir().join("wards").join("index.md").is_file());
        assert!(!state.vault_dir().join("wards").join("scratch").exists());
        assert!(!state.vault_dir().join("wards").join(".node_env").exists());
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
        assert!(state.memory_store().is_some());
        assert!(state.kg_store().is_some());
        assert!(state.distillation_repo().is_some());
        assert!(state.distiller().is_some());
        assert!(state.session_archiver().is_some());
        assert!(state.episode_store().is_some());
        assert!(state.kg_episode_store().is_some());
        assert!(state.cron_scheduler().is_none());
        assert!(state.bridge_bus().is_none());
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

        assert!(state.memory_store().is_some());
        assert!(state.kg_store().is_some());
        assert!(state.episode_store().is_some());
        assert!(state.wiki_store().is_some());
        assert!(state.procedure_store().is_some());
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

        assert_eq!(state.vault_dir(), *paths.vault_dir());
        assert!(state.memory_store().is_some());
        assert!(state.episode_store().is_some());
        assert!(state.wiki_store().is_some());
        assert!(state.procedure_store().is_some());
    }
}
