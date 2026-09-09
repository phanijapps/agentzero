//! # zbot-stores-sqlite
//!
//! SQLite-backed persistence for AgentZero. Implements the `zbot-stores`
//! traits (`KnowledgeGraphStore`, `MemoryFactStore`, etc.) plus the SQLite
//! connection pool, schema management, and per-table repositories that the
//! gateway uses.
//!
//! There is one SQLite crate for zbot persistence. See
//! `docs/architecture/architecture.md` for the broader persistence story.

// -- KnowledgeGraphStore impl + supporting glue (originally D6/D6b) -----------
pub mod kg;
mod knowledge_graph;

// -- Per-table stores ---------------------------------------------------------
pub mod belief_contradiction_store;
pub mod belief_store;
mod blocking;
pub mod compaction_repository;
pub mod compaction_store;
pub mod episode_repository;
pub mod episode_store;
pub mod kg_episode_repository;
pub mod knowledge_db;
pub mod knowledge_schema;
pub mod reindex;
pub mod sqlite_vec_loader;
pub mod vector_index;

// -- Public surface (D6b symbols) --------------------------------------------
pub use knowledge_graph::SqliteKgStore;

// -- Public surface -----------------------------------------------------------
pub use belief_contradiction_store::SqliteBeliefContradictionStore;
pub use belief_store::SqliteBeliefStore;
pub use compaction_repository::{Compaction, CompactionRepository, RunSummary};
pub use compaction_store::GatewayCompactionStore;
pub use episode_repository::{EpisodeRepository, SessionEpisode};
pub use episode_store::GatewayEpisodeStore;
pub use kg_episode_repository::{EpisodeSource, KgEpisode, KgEpisodeRepository};
pub use knowledge_db::KnowledgeDatabase;
pub use knowledge_schema::{
    drop_and_recreate_vec_tables_at_dim, list_vec_table_presence, REQUIRED_VEC_TABLES,
};
pub use vector_index::{SqliteVecIndex, VectorIndex};
pub use zbot_runtime_sqlite::system_profile;
pub use zbot_runtime_sqlite::DatabaseManager;
pub use zbot_runtime_sqlite::{
    DistillationRepository, DistillationRun, DistillationStats, GatewayDistillationStore,
    UndistilledSession,
};
