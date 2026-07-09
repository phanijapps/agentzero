//! `zbot-stores-traits` — dependency-light home for store traits.
//!
//! This crate holds the trait surface that consumers deep in the dep graph
//! (e.g. `agent-tools`) need to call without inheriting the full
//! `zbot-stores` transitive dependency on `knowledge-graph`.
//!
//! Re-exported from `zbot-stores` for the design-canonical
//! `zbot_stores::*` import paths.

pub mod auxiliary;
pub mod belief;
pub mod belief_contradiction;
pub mod compaction;
pub mod episodes;
pub mod kg_episodes;
pub mod memory_facts;
pub mod outbox;
pub mod procedures;
pub mod taxonomy;
pub mod wiki;

pub use auxiliary::{DistillationStore, GoalStore, RecallLogStore};
pub use belief::BeliefStore;
pub use belief_contradiction::BeliefContradictionStore;
pub use compaction::{CompactionRunSummary, CompactionStore};
// Re-export the Belief domain type so callers of BeliefStore only need
// one crate in their imports.
pub use episodes::{EpisodeStats, EpisodeStore, SessionEpisode, SuccessfulEpisode};
pub use kg_episodes::{KgEpisodeStatusCounts, KgEpisodeStore};
pub use memory_facts::{
    EmbeddingQueryIdentity, MemoryAggregateStats, MemoryFact, MemoryFactStore, MemoryHealthMetrics,
    SkillIndexRow, StrategyFactInsert, StrategyFactMatch,
};
pub use outbox::OutboxStore;
pub use procedures::{
    PatternProcedureInsert, PatternStep, Procedure, ProcedureStats, ProcedureStore,
    ProcedureSummary,
};
pub use taxonomy::{
    RecallTaxonomyExpander, RecallTaxonomyExpansion, RecallTaxonomyExpansionCandidate,
    RecallTaxonomyExpansionRequest,
};
pub use wiki::{WikiStats, WikiStore};
pub use zbot_stores_domain::{
    Belief, BeliefContradiction, ContradictionType, Resolution, ScoredBelief,
};
