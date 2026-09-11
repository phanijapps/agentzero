//! Memory subsystem configuration types. Owned by gateway-memory crate;
//! re-exported through gateway-services for backward compat.

pub mod llm_factory;
pub mod recall;
pub mod services;
pub mod sleep;
pub mod util;

pub use llm_factory::{CachedLlmClient, LlmClientConfig, MemoryLlmFactory};
pub use services::{MemoryServices, MemoryServicesConfig};
pub use util::{parse_llm_json, strip_code_fence};

pub use recall::context_atoms::{
    dropped_candidate_for_superseded_fact, scored_fact_to_context_atom,
    scored_item_to_context_atom, scored_items_to_context_atoms,
};
pub use recall::scored_item::{intent_boost, GoalLite, ItemKind, Provenance, ScoredItem};
pub use recall::{
    rerank::RerankConfig, MemoryRecall, RecallProviderScope, RecallSkosExpansionLimits,
    UnifiedRecallOutcome, UnifiedRecallReasonCode, UnifiedRecallScope, UnifiedRecallSourceState,
    UnifiedRecallSourceStatus, UnifiedRecallSourceSummary, UnifiedRecallTaxonomyCandidate,
    UnifiedRecallTaxonomyRelation, UnifiedRecallTaxonomyTrace,
};
pub use sleep::belief_engram::{
    BeliefConsolidation, BeliefConsolidationParts, BeliefContradictionConfig,
    ContradictionDetectionStats, ContradictionJudgeLlm, ContradictionJudgeResponse, JudgeDecision,
    LlmContradictionJudge,
};
pub use sleep::belief_engram::{
    BeliefSynthesisLlm, BeliefSynthesisStats, LlmBeliefSynthesizer, SynthesisLlmResponse,
    ZbotBeliefSink, ZbotBeliefSynthesizer, ZbotContradictionArm, ZbotContradictionDetector,
};
pub use sleep::belief_network_activity::{
    RecentBeliefNetworkActivity, TimestampedContradictionStats, TimestampedPropagationStats,
    TimestampedSynthesisStats, RECENT_CAPACITY as BELIEF_NETWORK_RECENT_CAPACITY,
};
pub use sleep::belief_propagator::{BeliefPropagationStats, BeliefPropagator};
pub use sleep::compactor::{CompactionStats, Compactor, PairwiseVerifier};
pub use sleep::conflict_resolver::{
    ConflictJudgeLlm, ConflictResolver, ConflictResponse, ConflictStats,
};
pub use sleep::decay::{DecayConfig, DecayEngine, KgDecayStats, PruneCandidate};
pub use sleep::orphan_archiver::{OrphanArchiver, OrphanArchiverStats};
pub use sleep::pattern_extractor::{
    PatternExtractLlm, PatternExtractor, PatternInput, PatternResponse, PatternStats,
};
pub use sleep::pruner::{PruneStats, Pruner};
pub use sleep::synthesizer::{
    SynthesisInput, SynthesisLlm, SynthesisResponse, SynthesisStats, Synthesizer,
};
pub use sleep::verifier::LlmPairwiseVerifier;
pub use sleep::worker::{CycleStats, SleepOps, SleepTimeWorker};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Canonical inspectable bytes for the Full Zbot memory V1 preset.
pub const ZBOT_RECOMMENDED_V1_MEMORY_JSON: &str =
    include_str!("../templates/zbot-recommended-v1-memory.json");

/// Canonical bytes written to `config/recall-config.json` for Full Zbot V1.
pub const ZBOT_RECOMMENDED_V1_RECALL_JSON: &str =
    include_str!("../templates/zbot-recommended-v1-recall.json");

// ============================================================================
// RECALL CONFIG
// Configurable recall priority engine with compiled defaults and JSON merge.
// Missing file → defaults, corrupted file → defaults, partial file → deep merge.
// The config file is NEVER auto-created or modified by the system.
// ============================================================================

/// Mid-session recall configuration — controls whether the system re-recalls
/// facts during an ongoing conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidSessionRecallConfig {
    pub enabled: bool,
    pub every_n_turns: usize,
    pub min_novelty_score: f64,
}

impl Default for MidSessionRecallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            every_n_turns: 5,
            min_novelty_score: 0.3,
        }
    }
}

/// Graph traversal configuration — controls how related facts are discovered
/// by walking knowledge-graph edges outward from directly recalled nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphTraversalConfig {
    pub enabled: bool,
    pub max_hops: u8,
    pub hop_decay: f64,
    pub max_graph_facts: usize,
    /// MEM-001 Part B-1 — minimum `kg_entities.confidence` for a hit to
    /// survive the recall step-4 graph-ANN filter. Hits below this
    /// threshold are dropped before scoring (low-confidence noise that
    /// would clutter recall without contributing). `0.0` disables the
    /// filter entirely — useful when migrating from older databases
    /// where confidence values haven't decayed yet.
    #[serde(default = "default_min_kg_confidence")]
    pub min_kg_confidence: f64,
}

fn default_min_kg_confidence() -> f64 {
    0.1
}

impl Default for GraphTraversalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_hops: 2,
            hop_decay: 0.6,
            max_graph_facts: 5,
            min_kg_confidence: default_min_kg_confidence(),
        }
    }
}

/// Temporal decay configuration — controls how fact relevance diminishes over
/// time, with per-category half-lives and pruning thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalDecayConfig {
    pub enabled: bool,
    pub half_life_days: HashMap<String, f64>,
    pub prune_threshold: f64,
    pub prune_after_days: u32,
}

impl Default for TemporalDecayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            half_life_days: HashMap::from([
                ("correction".to_string(), 90.0),
                ("strategy".to_string(), 60.0),
                ("domain".to_string(), 30.0),
                ("user".to_string(), 180.0),
                ("pattern".to_string(), 45.0),
                ("instruction".to_string(), 120.0),
            ]),
            prune_threshold: 0.05,
            prune_after_days: 30,
        }
    }
}

/// Predictive recall configuration — controls whether the system proactively
/// recalls facts based on patterns observed in similar past episodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictiveRecallConfig {
    pub enabled: bool,
    pub min_similar_successes: usize,
    pub predictive_boost: f64,
    pub max_episodes_to_check: usize,
}

impl Default for PredictiveRecallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_similar_successes: 2,
            predictive_boost: 1.3,
            max_episodes_to_check: 5,
        }
    }
}

/// Session offload configuration — controls when and how old session data is
/// archived to keep the active store lean.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOffloadConfig {
    pub enabled: bool,
    pub offload_after_days: u32,
    pub keep_session_metadata: bool,
    pub archive_path: String,
}

impl Default for SessionOffloadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            offload_after_days: 7,
            keep_session_metadata: true,
            archive_path: "data/archive".to_string(),
        }
    }
}

/// Knowledge-graph decay configuration — controls how entity and
/// relationship `confidence` is reduced over time based on `last_seen_at`.
/// Applied during the sleep-time cycle. Unlike `temporal_decay` (which is
/// per-category for `memory_facts`), KG decay uses a single half-life
/// for entities and another for relationships.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgDecayConfig {
    pub enabled: bool,
    pub entity_half_life_days: f64,
    pub relationship_half_life_days: f64,
    /// Floor — confidence never drops below this value.
    pub min_confidence: f64,
    /// Skip rows whose `last_seen_at` is within this many hours.
    pub skip_recent_hours: i64,
}

impl Default for KgDecayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            entity_half_life_days: 90.0,
            relationship_half_life_days: 90.0,
            min_confidence: 0.01,
            skip_recent_hours: 24,
        }
    }
}

/// Recall priority configuration — weights, limits, and thresholds that
/// control how memory facts and episodes are scored and retrieved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallConfig {
    pub category_weights: HashMap<String, f64>,
    pub ward_affinity_boost: f64,
    pub max_recall_tokens: usize,
    pub vector_weight: f64,
    pub bm25_weight: f64,
    pub max_facts: usize,
    pub max_episodes: usize,
    pub high_confidence_threshold: f64,
    /// Multiplier applied to the recall score of contradicted facts (0.0–1.0).
    pub contradiction_penalty: f64,
    /// Minimum score threshold — results scoring below this are suppressed.
    /// Prevents low-relevance facts from appearing for short generic queries.
    pub min_score: f64,
    pub mid_session_recall: MidSessionRecallConfig,
    pub graph_traversal: GraphTraversalConfig,
    pub temporal_decay: TemporalDecayConfig,
    pub predictive_recall: PredictiveRecallConfig,
    pub session_offload: SessionOffloadConfig,
    pub kg_decay: KgDecayConfig,
}

impl Default for RecallConfig {
    fn default() -> Self {
        let category_weights = HashMap::from([
            ("schema".to_string(), 1.6),
            // B-4: beliefs share weight with corrections — both are
            // distilled, agent-curated outputs. Schema (1.6) still wins
            // because schemas are the most opinionated knowledge type.
            ("belief".to_string(), 1.5),
            ("correction".to_string(), 1.5),
            ("strategy".to_string(), 1.4),
            ("user".to_string(), 1.3),
            ("instruction".to_string(), 1.2),
            ("domain".to_string(), 1.0),
            ("pattern".to_string(), 0.9),
            ("ward".to_string(), 0.8),
            ("skill".to_string(), 0.7),
            ("agent".to_string(), 0.7),
        ]);

        Self {
            category_weights,
            ward_affinity_boost: 1.3,
            max_recall_tokens: 3000,
            vector_weight: 0.7,
            bm25_weight: 0.3,
            max_facts: 10,
            max_episodes: 3,
            high_confidence_threshold: 0.9,
            contradiction_penalty: 0.7,
            min_score: 0.3,
            mid_session_recall: MidSessionRecallConfig::default(),
            graph_traversal: GraphTraversalConfig::default(),
            temporal_decay: TemporalDecayConfig::default(),
            predictive_recall: PredictiveRecallConfig::default(),
            session_offload: SessionOffloadConfig::default(),
            kg_decay: KgDecayConfig::default(),
        }
    }
}

impl RecallConfig {
    /// Construct the immutable Full Zbot recall V1 profile.
    ///
    /// Provisioning writes the canonical bundled bytes directly so `HashMap`
    /// iteration cannot drift.
    pub fn zbot_recommended_v1() -> Self {
        serde_json::from_str(ZBOT_RECOMMENDED_V1_RECALL_JSON)
            .expect("bundled Full Zbot recall V1 fixture must remain valid")
    }

    /// Load recall config from `{path}/config/recall-config.json`.
    ///
    /// - Missing file → compiled defaults (info log)
    /// - Corrupted file → compiled defaults (warning log)
    /// - Partial file → deep merge with defaults (user values win per key)
    ///
    /// This compatibility entry point accepts a vault root. New callers that
    /// already own a canonical path should use [`Self::load_from_file`].
    pub fn load_from_path(path: &Path) -> Self {
        Self::load_from_file(&path.join("config").join("recall-config.json"))
    }

    /// Load recall configuration from one resolved config file path.
    pub fn load_from_file(file_path: &Path) -> Self {
        if !file_path.exists() {
            tracing::info!(
                "No recall config at {} — using compiled defaults",
                file_path.display()
            );
            return Self::default();
        }

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Cannot read recall config at {} — using defaults: {}",
                    file_path.display(),
                    e
                );
                return Self::default();
            }
        };

        let overlay: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "Corrupted recall config at {} — using defaults: {}",
                    file_path.display(),
                    e
                );
                return Self::default();
            }
        };

        // Deep merge: serialize defaults to Value, merge overlay on top, deserialize back.
        let base = serde_json::to_value(Self::default()).expect("default config must serialize");
        let merged = deep_merge(base, overlay);

        match serde_json::from_value(merged) {
            Ok(config) => {
                tracing::info!(
                    "Loaded recall config from {} (merged with defaults)",
                    file_path.display()
                );
                config
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to deserialize merged recall config from {} — using defaults: {}",
                    file_path.display(),
                    e
                );
                Self::default()
            }
        }
    }

    /// Look up the weight for a memory category. Returns 1.0 for unknown categories.
    pub fn category_weight(&self, category: &str) -> f64 {
        *self.category_weights.get(category).unwrap_or(&1.0)
    }
}

/// Recursively merge two JSON values. Object keys from `overlay` overwrite
/// matching keys in `base`; nested objects are merged recursively.
/// Non-object values from `overlay` replace `base` entirely.
fn deep_merge(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                let base_val = base_map.remove(&key).unwrap_or(Value::Null);
                base_map.insert(key, deep_merge(base_val, value));
            }
            Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}

// ============================================================================
// MEMORY SETTINGS
// Background memory worker configuration — sleep cycle intervals.
// ============================================================================

/// Background memory worker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySettings {
    /// Durable memory provider selection. Engram is the only runtime semantic
    /// memory provider; SQLite remains only for conversations/execution state.
    #[serde(default)]
    pub provider: MemoryProviderSettings,
    /// Minimum hours between conflict-resolution LLM judge passes.
    /// Default: 24. Set to 0 to run on every sleep cycle (hourly).
    #[serde(default = "default_conflict_resolver_interval_hours")]
    pub conflict_resolver_interval_hours: u32,
    /// Belief Network synthesizer configuration. Phase B-1 of the
    /// reflective memory roadmap — opt-in (disabled by default).
    #[serde(default)]
    pub belief_network: BeliefNetworkConfig,
    /// Maximal Marginal Relevance diversity reranking. When enabled, the
    /// unified recall pipeline over-fetches `candidate_pool` items from
    /// RRF, then reranks via MMR before final truncation to the caller's
    /// budget. Default: disabled — when disabled, recall is byte-for-byte
    /// identical to pre-MMR behavior.
    #[serde(default)]
    pub mmr: MmrConfig,
    /// Cross-encoder rerank stage (the precision lever). Reranks the top
    /// `pool` fused candidates with a query-aware LLM score before
    /// MMR/truncation. Fail-open: any scorer error or timeout keeps the
    /// fused order. Kill-switch: `memory.rerank.enabled = false`.
    #[serde(default)]
    pub rerank: crate::recall::rerank::RerankConfig,
    /// Hierarchical-memory builder (Phase H-3). Opt-in (`enabled: false`
    /// by default). When enabled, an extra sleep-time worker clusters
    /// the current layer-N entities, synthesises layer-N+1 aggregates
    /// via an LLM, and writes LeanRAG-style inter-cluster relations
    /// between them. See `project_hierarchical_memory_plan.md`.
    #[serde(default)]
    pub hierarchy: HierarchySettings,
    /// Procedure recommendation gating. Controls when intent_analysis
    /// surfaces a learned procedure to the root agent and how strongly.
    /// Graduated tiers (promoted / advisory / tentative) trade off
    /// recall-similarity against accumulated `success_count` evidence —
    /// stronger framing for procedures that are both relevant AND proven.
    #[serde(default)]
    pub procedure_recommendation: ProcedureRecommendationConfig,
}

pub fn default_conflict_resolver_interval_hours() -> u32 {
    24
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            provider: MemoryProviderSettings::default(),
            rerank: crate::recall::rerank::RerankConfig::default(),
            conflict_resolver_interval_hours: default_conflict_resolver_interval_hours(),
            belief_network: BeliefNetworkConfig::default(),
            mmr: MmrConfig::default(),
            hierarchy: HierarchySettings::default(),
            procedure_recommendation: ProcedureRecommendationConfig::default(),
        }
    }
}

impl MemorySettings {
    /// Construct the immutable Full Zbot memory V1 profile.
    ///
    /// The contract test pins the complete bundled fixture independently from
    /// mutable defaults.
    pub fn zbot_recommended_v1() -> Self {
        serde_json::from_str(ZBOT_RECOMMENDED_V1_MEMORY_JSON)
            .expect("bundled Full Zbot memory V1 fixture must remain valid")
    }
}

/// Durable memory provider selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProviderSettings {
    /// Provider mode. Only Engram is a runtime provider.
    #[serde(default)]
    pub mode: MemoryProviderMode,
    /// Engram storage directory relative to the zbot data root, or absolute
    /// path confined under that root.
    #[serde(default = "default_engram_path")]
    pub engram_path: String,
    /// Tenant used for Engram scopes.
    #[serde(default = "default_memory_provider_tenant")]
    pub tenant: String,
    /// Where ward ids map in Engram scope.
    #[serde(default)]
    pub ward_scope_target: MemoryScopeTarget,
    /// Where belief partition ids map in Engram scope.
    #[serde(default)]
    pub partition_scope_target: MemoryScopeTarget,
    /// Adapter embedding compatibility behavior.
    #[serde(default)]
    pub embedding_mode: MemoryEmbeddingMode,
    /// Engram embedding provider identity for vector-space safety.
    #[serde(default)]
    pub embedding_provider: MemoryEmbeddingProviderSettings,
    /// Engram SQLite storage layout.
    #[serde(default)]
    pub sqlite_storage_layout: MemorySqliteStorageLayout,
    /// Migration execution mode used by adapter tooling.
    #[serde(default)]
    pub migration_mode: MemoryMigrationMode,
    /// Zbot-owned ontology/taxonomy governance policy.
    #[serde(default)]
    pub governance: MemoryGovernanceSettings,
}

impl Default for MemoryProviderSettings {
    fn default() -> Self {
        Self {
            mode: MemoryProviderMode::Engram,
            engram_path: default_engram_path(),
            tenant: default_memory_provider_tenant(),
            ward_scope_target: MemoryScopeTarget::Workspace,
            partition_scope_target: MemoryScopeTarget::Workspace,
            embedding_mode: MemoryEmbeddingMode::PreserveBytes,
            embedding_provider: MemoryEmbeddingProviderSettings::default(),
            sqlite_storage_layout: MemorySqliteStorageLayout::default(),
            migration_mode: MemoryMigrationMode::DryRun,
            governance: MemoryGovernanceSettings::default(),
        }
    }
}

/// Zbot-owned ontology/taxonomy governance settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryGovernanceSettings {
    /// Local ontology definition files under the trusted config root.
    #[serde(default)]
    pub ontology_definition_paths: Vec<String>,
    /// Local SKOS taxonomy definition files under the trusted config root.
    #[serde(default)]
    pub taxonomy_definition_paths: Vec<String>,
    /// Fallback ontology/taxonomy selection.
    #[serde(default)]
    pub default_selection: MemoryGovernanceSelection,
    /// Scoped selection overlays.
    #[serde(default)]
    pub overlays: Vec<MemoryGovernanceOverlay>,
    /// Ontology validation mode.
    #[serde(default)]
    pub validation_mode: MemoryGovernanceValidationMode,
    /// Behavior for unclassified records.
    #[serde(default)]
    pub allow_unclassified: MemoryAllowUnclassifiedPolicy,
    /// SKOS expansion limits for recall.
    #[serde(default)]
    pub skos_expansion: MemorySkosExpansionSettings,
}

impl Default for MemoryGovernanceSettings {
    fn default() -> Self {
        Self {
            ontology_definition_paths: Vec::new(),
            taxonomy_definition_paths: Vec::new(),
            default_selection: MemoryGovernanceSelection::default(),
            overlays: Vec::new(),
            validation_mode: MemoryGovernanceValidationMode::Advisory,
            allow_unclassified: MemoryAllowUnclassifiedPolicy::Allow,
            skos_expansion: MemorySkosExpansionSettings::default(),
        }
    }
}

/// Active ontology/taxonomy IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryGovernanceSelection {
    #[serde(default)]
    pub ontology_ids: Vec<String>,
    #[serde(default)]
    pub taxonomy_scheme_ids: Vec<String>,
}

/// Scoped governance overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryGovernanceOverlay {
    #[serde(default)]
    pub ward_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub selection: MemoryGovernanceSelection,
}

/// Ontology validation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryGovernanceValidationMode {
    #[default]
    Advisory,
    Disabled,
}

/// Unclassified record policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAllowUnclassifiedPolicy {
    #[default]
    Allow,
    Warn,
}

/// SKOS recall expansion limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySkosExpansionSettings {
    #[serde(default = "default_skos_expansion_depth")]
    pub max_depth: u8,
    #[serde(default = "default_skos_expansion_fan_out")]
    pub max_fan_out: u16,
    #[serde(default = "default_skos_expansion_candidates")]
    pub max_candidates: u16,
}

impl Default for MemorySkosExpansionSettings {
    fn default() -> Self {
        Self {
            max_depth: default_skos_expansion_depth(),
            max_fan_out: default_skos_expansion_fan_out(),
            max_candidates: default_skos_expansion_candidates(),
        }
    }
}

/// Durable memory provider mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProviderMode {
    /// Engram-backed adapter provider.
    #[default]
    Engram,
}

/// Engram scope target selected by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScopeTarget {
    /// Map to Engram workspace.
    #[default]
    Workspace,
    /// Map to Engram environment.
    Environment,
    /// Map to Engram subject.
    Subject,
}

/// Adapter embedding compatibility mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEmbeddingMode {
    /// Preserve zbot embedding bytes in adapter sidecars.
    #[default]
    PreserveBytes,
    /// Store only Engram embedding references.
    EngramRefs,
    /// Disable adapter-managed embeddings.
    Disabled,
}

/// Migration mode for adapter tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryMigrationMode {
    /// Report only.
    #[default]
    DryRun,
    /// Write after manifest acceptance.
    Apply,
}

/// Engram SQLite storage layout selected by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum MemorySqliteStorageLayout {
    /// Open one SQLite file per Engram store family.
    MultiFileDirectory,
    /// Open every Engram SQLite-backed store against one shared file.
    SingleFile { file_name: String },
}

impl Default for MemorySqliteStorageLayout {
    fn default() -> Self {
        Self::SingleFile {
            file_name: default_engram_data_file_name(),
        }
    }
}

/// Embedding provider identity used by Engram vector indexes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingProviderSettings {
    /// Provider family, e.g. fastembed, ollama, or openai.
    #[serde(default = "default_embedding_provider_type")]
    pub provider_type: String,
    /// Provider-specific model identifier.
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// Vector dimensions produced by the model.
    #[serde(default = "default_embedding_dimensions")]
    pub dimensions: u32,
    /// Prompt profile used for embeddings.
    #[serde(default = "default_embedding_prompt_profile")]
    pub prompt_profile: String,
    /// Normalization applied to embeddings, if any.
    #[serde(default)]
    pub normalization: Option<String>,
}

impl Default for MemoryEmbeddingProviderSettings {
    fn default() -> Self {
        Self {
            provider_type: default_embedding_provider_type(),
            model: default_embedding_model(),
            dimensions: default_embedding_dimensions(),
            prompt_profile: default_embedding_prompt_profile(),
            normalization: None,
        }
    }
}

fn default_engram_path() -> String {
    "engram".to_string()
}

fn default_engram_data_file_name() -> String {
    "engram_data.db".to_string()
}

fn default_memory_provider_tenant() -> String {
    "agentzero".to_string()
}

fn default_embedding_provider_type() -> String {
    "fastembed".to_string()
}

fn default_embedding_model() -> String {
    "BAAI/bge-small-en-v1.5".to_string()
}

fn default_embedding_dimensions() -> u32 {
    384
}

fn default_embedding_prompt_profile() -> String {
    "query".to_string()
}

fn default_skos_expansion_depth() -> u8 {
    1
}

fn default_skos_expansion_fan_out() -> u16 {
    8
}

fn default_skos_expansion_candidates() -> u16 {
    16
}

// ============================================================================
// PROCEDURE RECOMMENDATION CONFIG
// Three-tier gating for surfacing learned procedures in the root agent's
// system prompt. Higher tiers use stronger language; matches that don't
// clear any tier are silent.
// ============================================================================

/// Per-tier threshold pair. Procedure must clear BOTH the score floor (vec
/// similarity from `recall_procedures`) AND the success_count floor to
/// trigger that tier's framing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcedureTierConfig {
    /// Minimum vec-similarity score (strict >). Range typically 0.0-1.0.
    pub score_floor: f64,
    /// Minimum success_count (>=). Captures accumulated invocation evidence.
    pub success_floor: i32,
}

/// Procedure recommendation gating with graduated tiers.
///
/// Three tiers, evaluated top-down. The first tier whose floors are met
/// determines the framing in the root agent's system prompt:
///   * `promoted` — actionable "Recommended action: run_procedure(...)" block
///   * `advisory` — legacy "Proven Procedure Available" advisory text
///   * `tentative` — gentle "Possibly relevant procedure" FYI
///
/// A procedure that clears none of the tiers is silent (no surfacing at all).
/// Set `enabled: false` to disable surfacing entirely.
///
/// Tuning rationale:
///   * `promoted` is the explicit call to action with a `run_procedure(...)`
///     call template — reserved for high-confidence, well-evidenced matches.
///   * `advisory` is the historical surface; preserves prior behavior for
///     procedures with at least one corroborating session.
///   * `tentative` exists to bootstrap fresh procedures (sc=1 from distillation)
///     into the recommendation surface — the LLM sees the option but isn't
///     pushed. Successful invocation bumps sc and auto-promotes to advisory
///     on the next similar request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcedureRecommendationConfig {
    /// Master switch. Default: `true`.
    #[serde(default = "default_proc_rec_enabled")]
    pub enabled: bool,
    /// Strongest tier — call-to-action block with a literal `run_procedure(...)` template.
    #[serde(default = "default_promoted_tier")]
    pub promoted: ProcedureTierConfig,
    /// Middle tier — advisory block describing a proven procedure.
    #[serde(default = "default_advisory_tier")]
    pub advisory: ProcedureTierConfig,
    /// Lowest tier — gentle FYI for fresh procedures (sc=1) so they can mature.
    #[serde(default = "default_tentative_tier")]
    pub tentative: ProcedureTierConfig,
}

fn default_proc_rec_enabled() -> bool {
    true
}
fn default_promoted_tier() -> ProcedureTierConfig {
    ProcedureTierConfig {
        score_floor: 0.85,
        success_floor: 3,
    }
}
fn default_advisory_tier() -> ProcedureTierConfig {
    ProcedureTierConfig {
        score_floor: 0.70,
        success_floor: 2,
    }
}
fn default_tentative_tier() -> ProcedureTierConfig {
    ProcedureTierConfig {
        score_floor: 0.70,
        success_floor: 1,
    }
}

impl Default for ProcedureRecommendationConfig {
    fn default() -> Self {
        Self {
            enabled: default_proc_rec_enabled(),
            promoted: default_promoted_tier(),
            advisory: default_advisory_tier(),
            tentative: default_tentative_tier(),
        }
    }
}

/// Hierarchical-memory configuration (Phase H-3). Toggled by a master
/// `enabled` flag; when off, the sleep-time hierarchy build is never
/// constructed and the existing recall path is byte-for-byte unchanged.
///
/// All tuning knobs map 1:1 onto `sleep::hierarchy_engram::HierarchyConfig`
/// fields — see that struct's docs for the per-knob semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchySettings {
    /// Master switch. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Minimum hours between cycles. Default: 24.
    #[serde(default = "default_hierarchy_interval_hours")]
    pub interval_hours: u32,
    /// Hard cap on layers built per cycle. Default: 4.
    #[serde(default = "default_hierarchy_max_layers")]
    pub max_layers: u32,
    /// Target K-means cluster size (k ≈ n / target). Default: 20.
    #[serde(default = "default_hierarchy_cluster_target_size")]
    pub cluster_target_size: usize,
    /// Connectivity strength λ threshold above which inter-cluster
    /// relations are synthesised. Default: 3.
    #[serde(default = "default_inter_cluster_relation_threshold")]
    pub inter_cluster_relation_threshold: usize,
    /// Hard cap on LLM calls per cycle (sum of aggregate + relation
    /// synthesis). Default: 50.
    #[serde(default = "default_hierarchy_llm_budget")]
    pub llm_budget_per_cycle: u32,
}

pub fn default_hierarchy_interval_hours() -> u32 {
    24
}

pub fn default_hierarchy_max_layers() -> u32 {
    4
}

pub fn default_hierarchy_cluster_target_size() -> usize {
    20
}

pub fn default_inter_cluster_relation_threshold() -> usize {
    3
}

pub fn default_hierarchy_llm_budget() -> u32 {
    50
}

impl Default for HierarchySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: default_hierarchy_interval_hours(),
            max_layers: default_hierarchy_max_layers(),
            cluster_target_size: default_hierarchy_cluster_target_size(),
            inter_cluster_relation_threshold: default_inter_cluster_relation_threshold(),
            llm_budget_per_cycle: default_hierarchy_llm_budget(),
        }
    }
}

/// Belief Network configuration. Controls both the `BeliefSynthesizer`
/// (Phase B-1) and the `BeliefContradictionDetector` (Phase B-2)
/// sleep-time workers — a single block governs the whole reflective-memory
/// pillar so operators flip one master switch.
///
/// Disabled by default — operators opt in by setting `enabled: true`.
/// Throttled by `interval_hours` (default 24) to keep LLM cost bounded.
/// B-2 additions (`neighborhood_prefix_depth`, `contradiction_budget_per_cycle`)
/// only kick in when contradiction detection runs, which still gates on
/// the shared `enabled` flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeliefNetworkConfig {
    /// Master switch — covers B-1 synthesis, B-2 detection, and B-3
    /// confidence propagation. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Minimum hours between belief / contradiction cycles. Default: 24.
    #[serde(default = "default_belief_network_interval_hours")]
    pub interval_hours: u32,
    /// Phase B-2: how many dot-separated subject components define a
    /// contradiction-detection "neighborhood". `1` = top-level prefix
    /// (e.g. `user`); `2` = first two levels (`user.dietary`).
    /// Default: `1`.
    #[serde(default = "default_neighborhood_prefix_depth")]
    pub neighborhood_prefix_depth: usize,
    /// Phase B-2: maximum LLM judge calls per detection cycle. Pairs
    /// beyond the cap are skipped this cycle and may be picked up later.
    /// Default: 20.
    #[serde(default = "default_contradiction_budget_per_cycle")]
    pub contradiction_budget_per_cycle: usize,
    /// Phase B-3: threshold for fact-confidence-drop propagation. The
    /// DecayEngine fires `belief_invalidate` on a fact when EITHER its
    /// new confidence falls below this floor (and was above it before)
    /// OR the single-cycle drop exceeds this value. Default: `0.3`.
    #[serde(default = "default_fact_confidence_drop_threshold")]
    pub fact_confidence_drop_threshold: f64,
}

pub fn default_belief_network_interval_hours() -> u32 {
    24
}

pub fn default_neighborhood_prefix_depth() -> usize {
    1
}

pub fn default_contradiction_budget_per_cycle() -> usize {
    20
}

pub fn default_fact_confidence_drop_threshold() -> f64 {
    0.3
}

impl Default for BeliefNetworkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: default_belief_network_interval_hours(),
            neighborhood_prefix_depth: default_neighborhood_prefix_depth(),
            contradiction_budget_per_cycle: default_contradiction_budget_per_cycle(),
            fact_confidence_drop_threshold: default_fact_confidence_drop_threshold(),
        }
    }
}

// ============================================================================
// MMR CONFIG
// Maximal Marginal Relevance diversity reranking — post-rescore step that
// trades a little relevance for diversity in the final recalled set.
// Default-enabled (P2); disable via `memory.mmr.enabled = false`.
// ============================================================================

/// Configuration for Maximal Marginal Relevance (MMR) diversity reranking.
///
/// When `enabled: true`, the unified recall pipeline over-fetches
/// `candidate_pool` items from the RRF-fused candidate list, then reranks
/// them via the MMR algorithm:
///
/// ```text
/// next = argmax over remaining of:
///     lambda * relevance(c) - (1 - lambda) * max(sim(c, s) for s in Selected)
/// ```
///
/// Higher `lambda` favors relevance; lower favors diversity. The default
/// `0.6` weighs relevance 60% and diversity 40% — a balanced setting that
/// matches typical recommender-system tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MmrConfig {
    /// Master switch. Default: `true` — diversity reranking is on in
    /// production (P2 of the recall migration); the golden floors hold
    /// with it enabled.
    #[serde(default = "default_mmr_enabled")]
    pub enabled: bool,
    /// Relevance/diversity tradeoff in `[0.0, 1.0]`. `1.0` = pure relevance
    /// (degenerates to identity sort); `0.0` = pure diversity. Default: `0.6`.
    #[serde(default = "default_mmr_lambda")]
    pub lambda: f64,
    /// Over-fetch size from RRF before MMR reranks. The recall caller still
    /// receives at most `budget` items; this controls how many candidates
    /// MMR has to choose from. Default: `30`.
    #[serde(default = "default_mmr_candidate_pool")]
    pub candidate_pool: usize,
}

pub fn default_mmr_enabled() -> bool {
    true
}

pub fn default_mmr_lambda() -> f64 {
    0.6
}

pub fn default_mmr_candidate_pool() -> usize {
    30
}

impl Default for MmrConfig {
    fn default() -> Self {
        Self {
            enabled: default_mmr_enabled(),
            lambda: default_mmr_lambda(),
            candidate_pool: default_mmr_candidate_pool(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn default_config() {
        let config = RecallConfig::default();

        assert_eq!(config.category_weights.len(), 11);
        assert_eq!(config.category_weights["schema"], 1.6);
        assert_eq!(config.category_weights["belief"], 1.5);
        assert_eq!(config.category_weights["correction"], 1.5);
        assert_eq!(config.category_weights["strategy"], 1.4);
        assert_eq!(config.category_weights["user"], 1.3);
        assert_eq!(config.category_weights["instruction"], 1.2);
        assert_eq!(config.category_weights["domain"], 1.0);
        assert_eq!(config.category_weights["pattern"], 0.9);
        assert_eq!(config.category_weights["ward"], 0.8);
        assert_eq!(config.category_weights["skill"], 0.7);
        assert_eq!(config.category_weights["agent"], 0.7);

        assert_eq!(config.ward_affinity_boost, 1.3);
        assert_eq!(config.max_recall_tokens, 3000);
        assert_eq!(config.vector_weight, 0.7);
        assert_eq!(config.bm25_weight, 0.3);
        assert_eq!(config.max_facts, 10);
        assert_eq!(config.max_episodes, 3);
        assert_eq!(config.high_confidence_threshold, 0.9);

        assert!(config.mid_session_recall.enabled);
        assert_eq!(config.mid_session_recall.every_n_turns, 5);
        assert_eq!(config.mid_session_recall.min_novelty_score, 0.3);
    }

    #[test]
    fn load_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config = RecallConfig::load_from_path(tmp.path());

        // Should return defaults when file doesn't exist
        assert_eq!(config.max_recall_tokens, 3000);
        assert_eq!(config.max_facts, 10);
        assert_eq!(config.category_weights.len(), 11);
    }

    #[test]
    fn load_partial_override() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();

        // Override only a few fields — the rest should come from defaults
        let override_json = serde_json::json!({
            "max_facts": 25,
            "vector_weight": 0.8,
            "mid_session_recall": {
                "every_n_turns": 10
            },
            "category_weights": {
                "correction": 2.0,
                "custom_category": 1.1
            }
        });

        fs::write(
            config_dir.join("recall-config.json"),
            serde_json::to_string_pretty(&override_json).unwrap(),
        )
        .unwrap();

        let config = RecallConfig::load_from_path(tmp.path());

        // Overridden values
        assert_eq!(config.max_facts, 25);
        assert_eq!(config.vector_weight, 0.8);
        assert_eq!(config.mid_session_recall.every_n_turns, 10);

        // Deep merge: mid_session_recall fields not in overlay keep defaults
        assert!(config.mid_session_recall.enabled);
        assert_eq!(config.mid_session_recall.min_novelty_score, 0.3);

        // category_weights: overlay replaces the entire map (overlay wins at leaf level)
        // Since category_weights is an object, deep merge merges keys:
        // - "correction" overridden to 2.0
        // - "custom_category" added as 1.1
        // - other default keys preserved
        assert_eq!(config.category_weights["correction"], 2.0);
        assert_eq!(config.category_weights["custom_category"], 1.1);
        assert_eq!(config.category_weights["strategy"], 1.4); // default preserved

        // Non-overridden top-level values remain default
        assert_eq!(config.max_recall_tokens, 3000);
        assert_eq!(config.bm25_weight, 0.3);
        assert_eq!(config.max_episodes, 3);
        assert_eq!(config.high_confidence_threshold, 0.9);
        assert_eq!(config.ward_affinity_boost, 1.3);
    }

    #[test]
    fn load_corrupted_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();

        fs::write(
            config_dir.join("recall-config.json"),
            "this is not valid json {{{",
        )
        .unwrap();

        let config = RecallConfig::load_from_path(tmp.path());

        // Should fall back to defaults
        assert_eq!(config.max_recall_tokens, 3000);
        assert_eq!(config.max_facts, 10);
        assert_eq!(config.category_weights.len(), 11);
    }

    #[test]
    fn test_default_config_has_new_sections() {
        let config = RecallConfig::default();
        assert!(config.graph_traversal.enabled);
        assert_eq!(config.graph_traversal.max_hops, 2);
        assert_eq!(config.graph_traversal.hop_decay, 0.6);
        assert!(config.temporal_decay.enabled);
        assert_eq!(
            *config
                .temporal_decay
                .half_life_days
                .get("correction")
                .unwrap(),
            90.0
        );
        assert!(config.predictive_recall.enabled);
        assert_eq!(config.predictive_recall.predictive_boost, 1.3);
        assert!(config.session_offload.enabled);
        assert_eq!(config.session_offload.offload_after_days, 7);
    }

    #[test]
    fn graph_traversal_defaults_remain_enabled_depth_two() {
        // Pack A contract: these defaults must stay in sync with the activation spec.
        // See docs/superpowers/specs/2026-04-12-kg-activation-pack-a-design.md (Fix 6).
        let c = RecallConfig::default();
        assert!(
            c.graph_traversal.enabled,
            "graph_traversal.enabled default must remain true (Pack A contract)"
        );
        assert_eq!(
            c.graph_traversal.max_hops, 2,
            "graph_traversal.max_hops default must remain 2 (Pack A contract)"
        );
        assert!(
            c.graph_traversal.max_graph_facts >= 5,
            "graph_traversal.max_graph_facts default must be >= 5"
        );
    }

    #[test]
    fn test_partial_override_new_sections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("recall-config.json"),
            r#"{"graph_traversal": {"max_hops": 3}}"#,
        )
        .unwrap();
        let config = RecallConfig::load_from_path(dir.path());
        assert_eq!(config.graph_traversal.max_hops, 3); // overridden
        assert_eq!(config.graph_traversal.hop_decay, 0.6); // default preserved
        assert!(config.temporal_decay.enabled); // entirely default
    }

    // ------------------------------------------------------------------
    // MEM-001 Part B-1 — min_kg_confidence default + serde round-trip
    // ------------------------------------------------------------------

    #[test]
    fn min_kg_confidence_default_is_zero_point_one() {
        let c = RecallConfig::default();
        assert!(
            (c.graph_traversal.min_kg_confidence - 0.1).abs() < 1e-9,
            "min_kg_confidence default must be 0.1"
        );
    }

    #[test]
    fn min_kg_confidence_can_be_overridden_via_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("recall-config.json"),
            r#"{"graph_traversal": {"min_kg_confidence": 0.5}}"#,
        )
        .unwrap();
        let config = RecallConfig::load_from_path(dir.path());
        assert!(
            (config.graph_traversal.min_kg_confidence - 0.5).abs() < 1e-9,
            "min_kg_confidence override should take effect"
        );
        // Other fields stay at their compiled defaults.
        assert_eq!(config.graph_traversal.max_hops, 2);
    }

    #[test]
    fn min_kg_confidence_missing_key_falls_back_to_default() {
        // Older recall-config.json files won't have the field — verify
        // serde's `default = "..."` attribute fills in the compiled value
        // rather than failing to deserialize.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("recall-config.json"),
            r#"{"graph_traversal": {"max_hops": 3}}"#,
        )
        .unwrap();
        let config = RecallConfig::load_from_path(dir.path());
        assert!(
            (config.graph_traversal.min_kg_confidence - 0.1).abs() < 1e-9,
            "missing key should fall back to 0.1 default"
        );
    }

    #[test]
    fn category_weight_known_and_unknown() {
        let config = RecallConfig::default();

        // Known categories return their weight
        assert_eq!(config.category_weight("correction"), 1.5);
        assert_eq!(config.category_weight("agent"), 0.7);

        // Unknown categories return 1.0 fallback
        assert_eq!(config.category_weight("nonexistent"), 1.0);
        assert_eq!(config.category_weight(""), 1.0);
    }

    #[test]
    fn default_min_score_is_0_3() {
        let config = RecallConfig::default();
        assert_eq!(config.min_score, 0.3);
    }

    #[test]
    fn schema_category_weight_is_higher_than_correction() {
        let config = RecallConfig::default();
        let schema_w = config.category_weight("schema");
        let correction_w = config.category_weight("correction");
        assert!(
            schema_w > correction_w,
            "schema weight ({schema_w}) must exceed correction weight ({correction_w})"
        );
    }

    #[test]
    fn min_score_can_be_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("recall-config.json"), r#"{"min_score": 0.5}"#).unwrap();
        let config = RecallConfig::load_from_path(dir.path());
        assert_eq!(config.min_score, 0.5);
    }

    #[test]
    fn kg_decay_config_defaults() {
        let c = RecallConfig::default();
        assert!(c.kg_decay.enabled);
        assert_eq!(c.kg_decay.entity_half_life_days, 90.0);
        assert_eq!(c.kg_decay.relationship_half_life_days, 90.0);
        assert_eq!(c.kg_decay.min_confidence, 0.01);
        assert_eq!(c.kg_decay.skip_recent_hours, 24);
    }

    #[test]
    fn kg_decay_partial_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("recall-config.json"),
            r#"{"kg_decay": {"entity_half_life_days": 30.0}}"#,
        )
        .unwrap();
        let c = RecallConfig::load_from_path(dir.path());
        assert_eq!(c.kg_decay.entity_half_life_days, 30.0);
        // others remain default
        assert_eq!(c.kg_decay.relationship_half_life_days, 90.0);
        assert!(c.kg_decay.enabled);
    }

    #[test]
    fn default_conflict_resolver_interval_is_24() {
        let m = MemorySettings::default();
        assert_eq!(m.conflict_resolver_interval_hours, 24);
    }

    #[test]
    fn memory_settings_deserializes_partial() {
        let json = r#"{"conflictResolverIntervalHours": 6}"#;
        let m: MemorySettings = serde_json::from_str(json).unwrap();
        assert_eq!(m.conflict_resolver_interval_hours, 6);
        assert_eq!(m.provider.mode, MemoryProviderMode::Engram);
    }

    #[test]
    fn memory_provider_defaults_to_engram() {
        let m = MemorySettings::default();

        assert_eq!(m.provider.mode, MemoryProviderMode::Engram);
        assert_eq!(m.provider.engram_path, "engram");
        assert_eq!(m.provider.tenant, "agentzero");
        assert!(m.provider.governance.ontology_definition_paths.is_empty());
        assert!(m.provider.governance.taxonomy_definition_paths.is_empty());
        assert_eq!(
            m.provider.sqlite_storage_layout,
            MemorySqliteStorageLayout::SingleFile {
                file_name: "engram_data.db".to_string()
            }
        );
        assert_eq!(m.provider.embedding_provider.provider_type, "fastembed");
        assert_eq!(m.provider.embedding_provider.dimensions, 384);
    }

    #[test]
    fn memory_settings_deserializes_engram_provider_additively() {
        let json = r#"{
            "provider": {
                "mode": "engram",
                "engramPath": "memory/engram",
                "tenant": "zbot",
                "wardScopeTarget": "environment",
                "partitionScopeTarget": "subject",
                "embeddingMode": "engram_refs",
                "embeddingProvider": {
                    "providerType": "ollama",
                    "model": "nomic-embed-text",
                    "dimensions": 768,
                    "promptProfile": "query",
                    "normalization": "l2"
                },
                "sqliteStorageLayout": {
                    "kind": "single_file",
                    "fileName": "agent_memory.sqlite"
                },
                "migrationMode": "dry_run"
            }
        }"#;

        let m: MemorySettings = serde_json::from_str(json).unwrap();

        assert_eq!(m.provider.mode, MemoryProviderMode::Engram);
        assert_eq!(m.provider.engram_path, "memory/engram");
        assert_eq!(m.provider.tenant, "zbot");
        assert_eq!(m.provider.ward_scope_target, MemoryScopeTarget::Environment);
        assert_eq!(
            m.provider.partition_scope_target,
            MemoryScopeTarget::Subject
        );
        assert_eq!(m.provider.embedding_mode, MemoryEmbeddingMode::EngramRefs);
        assert_eq!(m.provider.embedding_provider.provider_type, "ollama");
        assert_eq!(m.provider.embedding_provider.model, "nomic-embed-text");
        assert_eq!(m.provider.embedding_provider.dimensions, 768);
        assert_eq!(
            m.provider.embedding_provider.normalization.as_deref(),
            Some("l2")
        );
        assert_eq!(
            m.provider.sqlite_storage_layout,
            MemorySqliteStorageLayout::SingleFile {
                file_name: "agent_memory.sqlite".to_string()
            }
        );
    }

    #[test]
    fn memory_settings_deserializes_governance_additively() {
        let json = r#"{
            "provider": {
                "governance": {
                    "ontologyDefinitionPaths": ["governance/base-ontology.json"],
                    "taxonomyDefinitionPaths": ["governance/base-taxonomy.json"],
                    "defaultSelection": {
                        "ontologyIds": ["zbot.base:v1"],
                        "taxonomySchemeIds": ["zbot.tasks:v1"]
                    },
                    "overlays": [{
                        "wardId": "ward-a",
                        "selection": {
                            "ontologyIds": ["ward.finance:v1"],
                            "taxonomySchemeIds": ["ward.finance:v1"]
                        }
                    }],
                    "validationMode": "disabled",
                    "allowUnclassified": "warn",
                    "skosExpansion": {
                        "maxDepth": 2,
                        "maxFanOut": 4,
                        "maxCandidates": 10
                    }
                }
            }
        }"#;

        let m: MemorySettings = serde_json::from_str(json).unwrap();

        assert_eq!(
            m.provider.governance.ontology_definition_paths,
            vec!["governance/base-ontology.json"]
        );
        assert_eq!(
            m.provider.governance.taxonomy_definition_paths,
            vec!["governance/base-taxonomy.json"]
        );
        assert_eq!(
            m.provider.governance.default_selection.ontology_ids,
            vec!["zbot.base:v1"]
        );
        assert_eq!(
            m.provider.governance.overlays[0].ward_id.as_deref(),
            Some("ward-a")
        );
        assert_eq!(
            m.provider.governance.validation_mode,
            MemoryGovernanceValidationMode::Disabled
        );
        assert_eq!(
            m.provider.governance.allow_unclassified,
            MemoryAllowUnclassifiedPolicy::Warn
        );
        assert_eq!(m.provider.governance.skos_expansion.max_depth, 2);
        assert_eq!(m.provider.governance.skos_expansion.max_fan_out, 4);
        assert_eq!(m.provider.governance.skos_expansion.max_candidates, 10);
    }

    #[test]
    fn belief_network_default_values() {
        let cfg = BeliefNetworkConfig::default();
        assert!(!cfg.enabled, "belief network must default to disabled");
        assert_eq!(cfg.interval_hours, 24);
        assert_eq!(cfg.neighborhood_prefix_depth, 1);
        assert_eq!(cfg.contradiction_budget_per_cycle, 20);
        assert!(
            (cfg.fact_confidence_drop_threshold - 0.3).abs() < 1e-9,
            "B-3 threshold defaults to 0.3"
        );
    }

    #[test]
    fn belief_network_legacy_json_back_compat() {
        // Existing settings.json from B-1 users only carries `enabled` +
        // `intervalHours`. Missing B-2 / B-3 fields must fall back to
        // defaults without failing deserialization.
        let json = r#"{"enabled": true, "intervalHours": 24}"#;
        let cfg: BeliefNetworkConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.interval_hours, 24);
        assert_eq!(cfg.neighborhood_prefix_depth, 1);
        assert_eq!(cfg.contradiction_budget_per_cycle, 20);
        assert!((cfg.fact_confidence_drop_threshold - 0.3).abs() < 1e-9);
    }

    #[test]
    fn belief_network_b3_only_legacy_json_keeps_b3_default() {
        // A settings.json that knows B-1 + B-2 but predates B-3 must
        // still parse and fall through to the 0.3 default for the new
        // field.
        let json = r#"{
            "enabled": true,
            "intervalHours": 24,
            "neighborhoodPrefixDepth": 2,
            "contradictionBudgetPerCycle": 50
        }"#;
        let cfg: BeliefNetworkConfig = serde_json::from_str(json).unwrap();
        assert!((cfg.fact_confidence_drop_threshold - 0.3).abs() < 1e-9);
    }

    #[test]
    fn belief_network_full_json_round_trips() {
        let json = r#"{
            "enabled": true,
            "intervalHours": 12,
            "neighborhoodPrefixDepth": 2,
            "contradictionBudgetPerCycle": 50,
            "factConfidenceDropThreshold": 0.45
        }"#;
        let cfg: BeliefNetworkConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.interval_hours, 12);
        assert_eq!(cfg.neighborhood_prefix_depth, 2);
        assert_eq!(cfg.contradiction_budget_per_cycle, 50);
        assert!((cfg.fact_confidence_drop_threshold - 0.45).abs() < 1e-9);
    }

    #[test]
    fn mmr_config_default_is_disabled() {
        let cfg = MmrConfig::default();
        assert!(cfg.enabled, "MMR now defaults to enabled (P2)");
        assert!((cfg.lambda - 0.6).abs() < f64::EPSILON);
        assert_eq!(cfg.candidate_pool, 30);
    }

    #[test]
    fn memory_settings_default_mmr_disabled() {
        let m = MemorySettings::default();
        assert!(m.mmr.enabled);
        assert!((m.mmr.lambda - 0.6).abs() < f64::EPSILON);
        assert_eq!(m.mmr.candidate_pool, 30);
    }

    #[test]
    fn mmr_config_deserializes_camel_case() {
        let json = r#"{
            "enabled": true,
            "lambda": 0.4,
            "candidatePool": 50
        }"#;
        let cfg: MmrConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.enabled);
        assert!((cfg.lambda - 0.4).abs() < f64::EPSILON);
        assert_eq!(cfg.candidate_pool, 50);
    }

    #[test]
    fn memory_settings_with_mmr_block_round_trips() {
        let json = r#"{
            "mmr": { "enabled": true, "lambda": 0.8 }
        }"#;
        let m: MemorySettings = serde_json::from_str(json).unwrap();
        assert!(m.mmr.enabled);
        assert!((m.mmr.lambda - 0.8).abs() < f64::EPSILON);
        // unspecified field keeps default
        assert_eq!(m.mmr.candidate_pool, 30);
    }

    #[test]
    fn mmr_legacy_json_back_compat() {
        // Existing settings.json from pre-MMR users won't have the block —
        // MemorySettings deserialization must still succeed with defaults.
        let json = r#"{"conflictResolverIntervalHours": 12}"#;
        let m: MemorySettings = serde_json::from_str(json).unwrap();
        // P2: MMR defaults on even when the block is absent; explicit
        // `{"mmr":{"enabled":false}}` remains the opt-out.
        assert!(m.mmr.enabled);
        assert!((m.mmr.lambda - 0.6).abs() < f64::EPSILON);
        assert_eq!(m.mmr.candidate_pool, 30);
    }

    #[test]
    fn hierarchy_settings_default_is_disabled() {
        let cfg = HierarchySettings::default();
        assert!(!cfg.enabled, "hierarchy must default to disabled");
        assert_eq!(cfg.interval_hours, 24);
        assert_eq!(cfg.max_layers, 4);
        assert_eq!(cfg.cluster_target_size, 20);
        assert_eq!(cfg.inter_cluster_relation_threshold, 3);
        assert_eq!(cfg.llm_budget_per_cycle, 50);
    }

    #[test]
    fn memory_settings_default_hierarchy_disabled() {
        let m = MemorySettings::default();
        assert!(!m.hierarchy.enabled);
    }

    #[test]
    fn hierarchy_settings_deserialises_camel_case() {
        let json = r#"{
            "enabled": true,
            "intervalHours": 12,
            "maxLayers": 3,
            "clusterTargetSize": 30,
            "interClusterRelationThreshold": 5,
            "llmBudgetPerCycle": 100
        }"#;
        let cfg: HierarchySettings = serde_json::from_str(json).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.interval_hours, 12);
        assert_eq!(cfg.max_layers, 3);
        assert_eq!(cfg.cluster_target_size, 30);
        assert_eq!(cfg.inter_cluster_relation_threshold, 5);
        assert_eq!(cfg.llm_budget_per_cycle, 100);
    }

    #[test]
    fn memory_settings_with_hierarchy_block_round_trips() {
        let json = r#"{
            "hierarchy": {
                "enabled": true,
                "maxLayers": 2
            }
        }"#;
        let m: MemorySettings = serde_json::from_str(json).unwrap();
        assert!(m.hierarchy.enabled);
        assert_eq!(m.hierarchy.max_layers, 2);
        // unspecified fields keep defaults
        assert_eq!(m.hierarchy.interval_hours, 24);
        assert_eq!(m.hierarchy.cluster_target_size, 20);
    }

    // AC4 — the V1 preset pins approved tuning and built-in identity.
    #[test]
    fn zbot_recommended_v1_pins_memory_tuning_and_builtin_embeddings() {
        let profile = MemorySettings::zbot_recommended_v1();
        let approved: serde_json::Value =
            serde_json::from_str(include_str!("../templates/zbot-recommended-v1-memory.json"))
                .unwrap();

        assert_eq!(serde_json::to_value(&profile).unwrap(), approved);
        assert!(profile.belief_network.enabled);
        assert_eq!(profile.belief_network.interval_hours, 0);
        assert_eq!(profile.belief_network.contradiction_budget_per_cycle, 200);
        assert!(profile.mmr.enabled);
        assert!(profile.hierarchy.enabled);
        assert_eq!(profile.hierarchy.interval_hours, 0);
        assert!((profile.procedure_recommendation.tentative.score_floor - 0.55).abs() < 1e-9);
        assert_eq!(
            profile.provider.embedding_provider.provider_type,
            "fastembed"
        );
        assert_eq!(
            profile.provider.embedding_provider.model,
            "bge-small-en-v1.5"
        );
        assert_eq!(profile.provider.embedding_provider.dimensions, 384);
        assert_eq!(profile.provider.embedding_provider.prompt_profile, "query");
        assert_eq!(
            profile.provider.governance.default_selection.ontology_ids,
            ["zbot.base:v1"]
        );
        assert_eq!(
            profile
                .provider
                .governance
                .default_selection
                .taxonomy_scheme_ids,
            ["zbot.general:v1"]
        );
    }

    // AC5 — the versioned recall profile is materializable and inspectable.
    #[test]
    fn zbot_recommended_v1_materializes_recall_defaults() {
        let approved_bytes = include_str!("../templates/zbot-recommended-v1-recall.json");
        let approved: RecallConfig = serde_json::from_str(approved_bytes).unwrap();
        let profile = RecallConfig::zbot_recommended_v1();
        assert_eq!(
            serde_json::to_value(profile).unwrap(),
            serde_json::to_value(approved).unwrap()
        );
        assert_eq!(RecallConfig::default().max_recall_tokens, 3000);

        let json = ZBOT_RECOMMENDED_V1_RECALL_JSON.to_string();
        assert_eq!(json.as_bytes(), approved_bytes.as_bytes());
        let parsed: RecallConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_recall_tokens, 3000);
        assert_eq!(parsed.max_facts, 10);
        assert_eq!(parsed.max_episodes, 3);
        assert!(parsed.mid_session_recall.enabled);
        assert!(parsed.graph_traversal.enabled);
        assert!(parsed.predictive_recall.enabled);
    }

    // AC5 — bundled definitions expose the approved governance IDs.
    #[test]
    fn bundled_governance_definitions_have_approved_ids() {
        fn bundled_governance() -> (&'static str, &'static str) {
            (
                include_str!("../../templates/governance/base-ontology.json"),
                include_str!("../../templates/governance/base-taxonomy.json"),
            )
        }

        let (ontology, taxonomy) = bundled_governance();
        let ontology: serde_json::Value = serde_json::from_str(ontology).unwrap();
        let taxonomy: serde_json::Value = serde_json::from_str(taxonomy).unwrap();
        assert_eq!(ontology["kind"], "zbot.ontology");
        assert_eq!(ontology["ontologyId"], "zbot.base:v1");
        assert_eq!(taxonomy["kind"], "zbot.skos_taxonomy");
        assert_eq!(taxonomy["schemeId"], "zbot.general:v1");
    }
}
