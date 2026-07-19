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
mod blocking;
pub mod bootstrap;
pub mod kg;
mod knowledge_graph;
pub mod reindex;

// -- Per-table stores ---------------------------------------------------------
pub mod auxiliary_stores;
pub mod belief_contradiction_store;
pub mod belief_store;
pub mod compaction_repository;
pub mod compaction_store;
pub mod episode_repository;
pub mod episode_store;
pub mod goal_repository;
pub mod kg_episode_repository;
pub mod kg_episode_store;
pub mod knowledge_db;
pub mod knowledge_schema;
pub mod memory_fact_store;
pub mod memory_repository;
pub mod procedure_repository;
pub mod procedure_store;
pub mod recall_log_repository;
pub mod sqlite_vec_loader;
pub mod vector_index;
pub mod wiki_repository;
pub mod wiki_store;

// -- Public surface (D6b symbols) --------------------------------------------
pub use knowledge_graph::SqliteKgStore;

// -- Public surface -----------------------------------------------------------
pub use auxiliary_stores::{GatewayGoalStore, GatewayRecallLogStore};
pub use belief_contradiction_store::SqliteBeliefContradictionStore;
pub use belief_store::SqliteBeliefStore;
pub use compaction_repository::{Compaction, CompactionRepository, RunSummary};
pub use compaction_store::GatewayCompactionStore;
pub use episode_repository::{EpisodeRepository, SessionEpisode};
pub use episode_store::GatewayEpisodeStore;
pub use goal_repository::{Goal, GoalRepository};
pub use kg_episode_repository::{EpisodeSource, KgEpisode, KgEpisodeRepository};
pub use kg_episode_store::GatewayKgEpisodeStore;
pub use knowledge_db::KnowledgeDatabase;
pub use knowledge_schema::{
    drop_and_recreate_vec_tables_at_dim, list_vec_table_presence, REQUIRED_VEC_TABLES,
};
pub use memory_fact_store::GatewayMemoryFactStore;
pub use memory_repository::{MemoryFact, MemoryRepository, ScoredFact, SkillIndexRow};
pub use procedure_repository::{Procedure, ProcedureRepository};
pub use procedure_store::GatewayProcedureStore;
pub use recall_log_repository::RecallLogRepository;
pub use vector_index::{SqliteVecIndex, VectorIndex};
pub use wiki_repository::{WardWikiRepository, WikiArticle, WikiHit};
pub use wiki_store::GatewayWikiStore;
pub use zbot_runtime_sqlite::system_profile;
pub use zbot_runtime_sqlite::DatabaseManager;
pub use zbot_runtime_sqlite::{
    DistillationRepository, DistillationRun, DistillationStats, GatewayDistillationStore,
    UndistilledSession,
};

/// Canonical alias for the SQLite `MemoryFactStore` impl. Mirrors the
/// `Sqlite*` naming used by `SqliteKgStore` — the persistence factory in
/// `gateway/src/state/persistence_factory.rs` constructs the store via this
/// alias.
pub use memory_fact_store::GatewayMemoryFactStore as SqliteMemoryStore;
