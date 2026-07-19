# zbot-stores-sqlite

SQLite-backed persistence for AgentZero. The single production backend implementing all `zbot-stores` traits plus the connection pool, schema management, and the vector index.

There is one SQLite crate for zbot persistence.

## Key Exports

```rust
// Connection pool + schema
pub use connection::DatabaseManager;        // r2d2 pool, WAL mode, 8 connections

// KnowledgeGraphStore impl
pub use knowledge_graph::SqliteKgStore;

// MemoryFactStore impl (two names, same type)
pub use memory_fact_store::GatewayMemoryFactStore;
pub use memory_fact_store::GatewayMemoryFactStore as SqliteMemoryStore;

// Conversation / execution repositories
pub use repository::{ConversationRepository, Message};
pub use episode_repository::{EpisodeRepository, SessionEpisode};
pub use memory_repository::{MemoryRepository, MemoryFact, ScoredFact};

// Knowledge DB (unified KG + embedding handle)
pub use knowledge_db::KnowledgeDatabase;

// Vector index (sqlite-vec)
pub use vector_index::{SqliteVecIndex, VectorIndex};
pub use knowledge_schema::{drop_and_recreate_vec_tables_at_dim, REQUIRED_VEC_TABLES};

// Gateway store wrappers (implement zbot-stores-traits)
pub use auxiliary_stores::{GatewayDistillationStore, GatewayGoalStore, GatewayRecallLogStore};
pub use compaction_store::GatewayCompactionStore;
pub use episode_store::GatewayEpisodeStore;
pub use kg_episode_store::GatewayKgEpisodeStore;
pub use procedure_store::GatewayProcedureStore;
pub use wiki_store::GatewayWikiStore;
pub use wiki_repository::{WardWikiRepository, WikiArticle, WikiHit};
```

## Intra-Repo Dependencies

- `zbot-stores`, `zbot-stores-traits`, `zbot-stores-domain` — traits and types being implemented
- `knowledge-graph` — `Entity`, `Relationship`, etc. for KG storage
- `api-logs`, `execution-state` — implement `DbProvider` traits for those services
- `agent-runtime`, `gateway-services`, `agent-primitives` — auxiliary dependencies for bootstrapping

## Database

- **Engine**: SQLite via `rusqlite` (bundled) + `r2d2_sqlite`
- **Extensions**: `sqlite-vec` for vector similarity search
- **Pool**: 8 connections, WAL mode (allows concurrent readers)
- **Migrations**: Applied at startup via `bootstrap::run_migrations()`

## Notes

- The gateway wires all stores in `gateway/src/state/persistence_factory.rs`.
- Vector tables are dimension-specific; call `drop_and_recreate_vec_tables_at_dim()` when embedding model changes.
- `reindex.rs` provides background re-indexing logic for the knowledge graph.
