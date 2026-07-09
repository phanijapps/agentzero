//! Pure-data domain types for the AgentZero persistence layer.
//!
//! This crate is dep-light by design — only `serde`. It is the home for
//! every value type that crosses the persistence boundary (in either
//! direction) and that any backend impl needs to round-trip.
//!
//! Adding a new domain type? It belongs here when:
//! - It has no methods that touch a database, file, or network
//! - Multiple crates need to construct or read it
//! - Backend impls (SQLite, SurrealDB, Postgres, …) all serialize the same shape
//!
//! Things that do NOT belong here:
//! - Repository structs (`MemoryRepository`, `KnowledgeDatabase`) — those are
//!   storage logic, they live in backend impl crates.
//! - Trait surfaces (`MemoryFactStore`, `KnowledgeGraphStore`) — those live in
//!   `zbot-stores-traits`.
//! - HTTP request/response shapes — those live in the gateway HTTP layer
//!   (they can derive From/Into the domain types here).

pub mod belief;
pub mod belief_contradiction;
pub mod distillation_ops;
pub mod goal;
pub mod kg_episode;
pub mod kg_ops;
pub mod memory_fact;
pub mod procedure;
pub mod route_hint;
pub mod session_episode;
pub mod wiki;

pub use belief::{Belief, ScoredBelief};
pub use belief_contradiction::{BeliefContradiction, ContradictionType, Resolution};
pub use distillation_ops::{DistillationStats, UndistilledSession};
pub use goal::Goal;
pub use kg_episode::{EpisodeSource, KgEpisode};
pub use kg_ops::{
    AggregateSummary, DecayCandidate, DuplicateCandidate, EntityNameEmbeddingHit, GraphView,
    HierarchySummary, InterClusterRelationHit, RelationshipContext, StrategyCandidate,
};
pub use memory_fact::{MemoryFact, ScoredFact, StrategyFactInsert, StrategyFactMatch};
pub use procedure::{PatternProcedureInsert, PatternStep, Procedure, ProcedureSummary};
pub use route_hint::{RouteHint, RouteSourceKind};
pub use session_episode::{ScoredEpisode, SessionEpisode, SuccessfulEpisode};
pub use wiki::{WikiArticle, WikiHit};
