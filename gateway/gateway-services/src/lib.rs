//! # Gateway Services
//!
//! Config-based services for the AgentZero gateway.
//!
//! Provides file-backed services for managing:
//! - Agent configurations
//! - LLM provider configurations
//! - MCP server configurations
//! - Skill configurations
//! - Application settings (tools, logging)
//! - Agent registry (delegation permissions)
//! - Plugin configurations

pub mod agent_registry;
pub mod agents;
pub mod embedding_service;
pub mod lang_config;
pub mod llm_factory;
pub mod logging;
pub mod mcp;
pub mod mcp_oauth;
pub mod models;
pub mod ollama_client;
pub mod paths;
pub mod plugin_service;
pub mod providers;
pub mod recall_config;
pub mod settings;
pub mod skills;
pub mod ward_curator;
pub mod ward_layout;
pub mod ward_usage;
pub mod watcher;

#[cfg(windows)]
mod windows_file;

pub use agent_registry::AgentRegistry;
pub use agents::{validate_configured_agent_id, AgentService};
pub use embedding_service::{
    curated_lookup, CuratedModel, EmbeddingBackend, EmbeddingConfig, EmbeddingService, Health,
    LiveEmbeddingClient, OllamaConfig, CURATED_MODELS,
};
pub use lang_config::{load_all_lang_configs, load_lang_config, LangConfig};
pub use llm_factory::{provider_client, select_provider};
pub use logging::LogSettings;
pub use mcp::McpService;
pub use mcp_oauth::{McpOAuthService, McpOAuthStartResponse};
pub use models::ModelRegistry;
pub use ollama_client::OllamaClient;
pub use paths::{SharedVaultPaths, VaultPaths};
pub use plugin_service::PluginService;
pub use providers::ProviderService;
pub use recall_config::{KgDecayConfig, RecallConfig};
pub use settings::{
    AppSettings, ChatConfig, CommissioningSettings, CommissioningState, CuratorConfig,
    DistillationConfig, ExecutionSettings, IntentAnalysisConfig, MemorySettings, MultimodalConfig,
    OrchestratorConfig, PresentationSettings, SemanticProfile, SemanticProvisioning,
    SettingsService, UserProfile,
};
pub use skills::{
    Skill, SkillFileInfo, SkillFrontmatter, SkillService, SkillSource, WardAgentsMdConfig,
    WardSetup,
};
pub use ward_curator::{
    AppliedAction, ApplyStatus, CleanupReport, CleanupRequest, ConsolidateRequest,
    ConsolidationAction, ConsolidationPlan, ConsolidationReport, RestoreReport, RestoreRequest,
    Transition, WardCandidate, WardCurator,
};
pub use ward_layout::{
    create_ward_from_archetype, create_ward_from_template, lint_ward, load_bounded_vault_utf8_file,
    load_ward_agent_template, load_ward_archetype_bundle, load_ward_layout, load_ward_layout_bytes,
    publish_tree_no_replace, rollback_created_ward, seed_default_ward_agent_template,
    seed_default_ward_archetypes, seed_default_ward_layout_template, BoundedFileError,
    CompiledWardLayout, CreatedWard, FindingCategory, LayoutError, LintFinding,
    LoadedWardArchetype, LoadedWardLayout, NodeFormat, NodeKind, RuleError, RuleNode, SeedOutcome,
    WardCreateError, WardLayoutDocument, WardLintReport, WardStarterFile,
    MAX_WARD_ARCHETYPE_STARTER_DEPTH, MAX_WARD_ARCHETYPE_STARTER_FILES,
    MAX_WARD_ARCHETYPE_STARTER_FILE_BYTES, MAX_WARD_ARCHETYPE_STARTER_TOTAL_BYTES,
    WARD_AGENT_TEMPLATE_MAX_BYTES,
};
pub use ward_usage::{WardProvenance, WardRecord, WardState, WardUsage, WardUsageMap};
pub use watcher::{FileWatcher, WatchConfig};
