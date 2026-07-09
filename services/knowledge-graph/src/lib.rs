//! # Knowledge Graph
//!
//! Extracts and stores entities and relationships from conversations.
//!
//! Features:
//! - Entity extraction (people, places, organizations, concepts)
//! - Relationship extraction (works for, located in, related to, etc.)
//! - LLM-powered smart extraction
//!
//! ## Storage layer
//!
//! The SQLite-coupled storage (`storage`, `traversal`, `causal`, `service`) was
//! relocated to `zbot-stores-sqlite::kg` during Slice D6b of the persistence
//! refactor. Consumers should import those types via `zbot_stores_sqlite::kg::*`.
//! See `docs/architecture/future-state/db-provider-portability.md`.

pub mod error;
pub mod extractor;
pub mod resolver;
pub mod types;

pub use error::{GraphError, GraphResult};
pub use extractor::EntityExtractor;
pub use resolver::{normalize_name, resolve, MatchReason, ResolveOutcome};
pub use types::{
    Direction, Entity, EntityType, EntityWithConnections, ExtractedKnowledge, GraphStats,
    NeighborInfo, Relationship, RelationshipType, Subgraph,
};
