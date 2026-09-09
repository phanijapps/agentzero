//! `zbot-stores` — backend-agnostic persistence interfaces for AgentZero.
//!
//! This crate defines store traits and shared types only. It pulls in NO
//! database drivers. The concrete implementation lives in
//! `zbot-stores-sqlite`; alternate backends drop in by implementing the
//! same traits.

pub mod belief;
pub mod belief_contradiction;
pub mod error;
pub mod extracted;
pub mod knowledge_graph;
pub mod memory_facts;
pub mod types;

pub use belief::{Belief, BeliefStore, ScoredBelief};
pub use belief_contradiction::{
    BeliefContradiction, BeliefContradictionStore, ContradictionType, Resolution,
};
pub use error::{GraphStoreError, GraphStoreResult};
pub use extracted::ExtractedKnowledge;
pub use knowledge_graph::{
    AggregateSummary, DecayCandidate, DuplicateCandidate, EntityNameEmbeddingHit,
    EntityWithEmbedding, GraphView, HierarchySummary, InterClusterRelationHit, KgNodesForEpisodes,
    KnowledgeGraphStore, LcaPath, RelationshipContext, StrategyCandidate, WeightedTraversalHit,
};
pub use memory_facts::{
    EmbeddingQueryIdentity, MemoryAggregateStats, MemoryFactStore, MemoryFactWriteRequest,
    MemoryHealthMetrics, SkillIndexRow, StrategyFactInsert, StrategyFactMatch,
};
pub use types::*;
pub use zbot_stores_traits::{StoreError, StoreResult};
