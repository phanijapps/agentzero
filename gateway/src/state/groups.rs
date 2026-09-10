//! The six focused state groups (W2 of the gateway decompose).
//!
//! `AppState` is the system's dependency surface; these structs give it
//! structure: every consumer should declare the ONE group it needs rather
//! than the whole world. W2 introduces them alongside the flat fields
//! (constructed from the same Arcs — clones are cheap, breadth is not);
//! later waves migrate consumers and delete the flat surface.
//!
//! Grouping follows the deck (docs/gateway-decompose-deck.html slide 3):
//! stores (the engram bundle + conversation stores), services (capability/
//! config services), execution (engine handles), transport (gateway edges),
//! workers (background loops), vault (environment).

use std::path::PathBuf;
use std::sync::Arc;

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
use zbot_engram_adapter::GovernanceCapabilityHealth;
use zbot_runtime_sqlite::{DatabaseManager, DistillationRepository};

/// Stores group — see module docs.
#[derive(Clone)]
pub struct StoresState {
    pub memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>>,
    pub goal_store: Option<Arc<dyn zbot_stores_traits::GoalStore>>,
    pub distillation_repo: Option<Arc<DistillationRepository>>,
    pub episode_store: Option<Arc<dyn zbot_stores_traits::EpisodeStore>>,
    pub wiki_store: Option<Arc<dyn zbot_stores_traits::WikiStore>>,
    pub procedure_store: Option<Arc<dyn zbot_stores_traits::ProcedureStore>>,
    pub kg_episode_store: Option<Arc<dyn zbot_stores_traits::KgEpisodeStore>>,
    pub kg_store: Option<Arc<dyn knowledge_graph::kg_trait::KnowledgeGraphStore>>,
    pub governance_health: Option<GovernanceCapabilityHealth>,
    pub compaction_store: Option<Arc<dyn zbot_stores_traits::CompactionStore>>,
    pub belief_store: Option<Arc<dyn zbot_stores_traits::BeliefStore>>,
    pub belief_contradiction_store: Option<Arc<dyn zbot_stores_traits::BeliefContradictionStore>>,
    pub belief_network_activity: Option<Arc<gateway_memory::RecentBeliefNetworkActivity>>,
    pub messages: Arc<dyn zbot_conversation::MessageStore>,
    pub session_meta: Arc<dyn zbot_conversation::SessionMetaStore>,
    pub checkpoints: Arc<dyn zbot_conversation::CheckpointStore>,
    pub slim_logs: Arc<dyn zbot_trace::SlimLogStore>,
}

/// Services group — see module docs.
#[derive(Clone)]
pub struct ServicesState {
    pub agents: Arc<AgentService>,
    pub skills: Arc<SkillService>,
    pub provider_service: Arc<ProviderService>,
    pub mcp_service: Arc<McpService>,
    pub settings: Arc<SettingsService>,
    pub log_service: Arc<LogService<DatabaseManager>>,
    pub state_service: Arc<StateService<DatabaseManager>>,
    pub model_registry: Arc<ModelRegistry>,
    pub embedding_service: Arc<EmbeddingService>,
}

/// Execution group — see module docs.
#[derive(Clone)]
pub struct ExecutionState {
    pub runtime: Arc<RuntimeService>,
    pub event_bus: Arc<EventBus>,
    pub hook_registry: Option<Arc<HookRegistry>>,
    pub delegation_registry: Arc<DelegationRegistry>,
    pub autonomy: Arc<dyn zbot_conversation::AutonomyStore>,
    pub trace_analytics: Arc<zbot_trace::TraceAnalytics>,
    pub ingestion_queue: Option<Arc<gateway_execution::ingest::IngestionQueue>>,
    pub ingestion_backpressure: Option<Arc<gateway_execution::ingest::Backpressure>>,
}

/// Transport group — see module docs.
#[derive(Clone)]
pub struct TransportState {
    pub durable_work_transport: Arc<gateway_bus::LocalWorkTransport>,
    pub connector_registry: Arc<ConnectorRegistry>,
    pub bridge_registry: Arc<gateway_bridge::BridgeRegistry>,
    pub bridge_outbox: Arc<gateway_bridge::OutboxRepository>,
    pub bridge_bus: Option<Arc<dyn gateway_bus::GatewayBus>>,
    pub plugin_manager: Arc<gateway_bridge::PluginManager>,
    pub advertiser: std::sync::Arc<dyn discovery::Advertiser>,
    pub advertise_handle: std::sync::Arc<std::sync::Mutex<Option<discovery::AdvertiseHandle>>>,
}

/// Workers group — see module docs.
#[derive(Clone)]
pub struct WorkersState {
    pub durable_work_store: Arc<dyn WorkStore>,
    pub distiller: Option<Arc<distillation::SessionDistiller>>,
    pub cron_scheduler: Option<Arc<CronScheduler>>,
    pub session_archiver: Option<Arc<SessionArchiver>>,
    pub sleep_time_worker: Option<Arc<gateway_memory::sleep::SleepTimeWorker>>,
}

/// Vault group — see module docs.
#[derive(Debug, Clone)]
pub struct VaultState {
    pub paths: SharedVaultPaths,
    pub vault_dir: PathBuf,
}
