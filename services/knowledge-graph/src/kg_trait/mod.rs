//! The store-trait module: backend-agnostic KG persistence contract plus
//! its supporting types, moved from the retired `zbot-stores` facade.
//!
//! The trait lives here — next to the `types` it is expressed in — instead
//! of the facade, so the facade crate could be deleted without pulling the
//! `knowledge-graph` types into `zbot-stores-traits` (which stays
//! dependency-light for deep consumers like `agent-tools`).

pub mod error;
pub mod extracted;
pub mod kg_types;
mod store;

pub use error::{GraphStoreError, GraphStoreResult};
pub use extracted::ExtractedKnowledge;
pub use kg_types::{
    ArchivableEntity, EntityId, KgStats, Neighbor, ReindexReport, RelationshipId, ResolveOutcome,
    StoreOutcome, TraversalHit, VecIndexHealth,
};
pub use store::{
    EntityWithEmbedding, KgNodesForEpisodes, KnowledgeGraphStore, LcaPath, WeightedTraversalHit,
};

// Port request/response shapes re-exported at the trait surface so
// `knowledge_graph::{DuplicateCandidate, DecayCandidate, ...}` keeps the
// import paths callers used via `zbot_stores::*`.
pub use store::{
    AggregateSummary, DecayCandidate, DuplicateCandidate, EntityNameEmbeddingHit, GraphView,
    HierarchySummary, InterClusterRelationHit, RelationshipContext, StrategyCandidate,
};
