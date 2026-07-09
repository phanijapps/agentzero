# Stores

Backend-agnostic persistence layer for AgentZero. Separates domain types, trait surfaces, and concrete implementations so alternate database backends can be added without changing consumers.

## Crates

| Crate | Purpose |
|-------|---------|
| `zbot-stores-domain` | Pure-data domain types (serde only, no DB) |
| `zbot-stores-traits` | Store trait surfaces — lightweight, no DB drivers |
| `zbot-stores` | Re-exports traits + types; also holds `KnowledgeGraphStore` / `MemoryFactStore` traits |
| `zbot-stores-sqlite` | SQLite implementation (rusqlite + sqlite-vec + r2d2) — the only concrete backend |
| `zbot-stores-conformance` | Cross-impl behavioral test harness |

## Dependency Order

```
zbot-stores-domain  (serde only)
    └── zbot-stores-traits
          └── zbot-stores  (adds KnowledgeGraphStore, MemoryFactStore traits + types)
                └── zbot-stores-sqlite  (concrete impl + r2d2 pool + vector index)
                      └── zbot-stores-conformance  (test harness, no production usage)
```

## Why the Split

`zbot-stores-traits` exists to break a dep cycle: `agent-tools` (deep in the graph, below
`gateway`) needs `MemoryFactStore` but cannot depend on `zbot-stores` (which pulls in
`knowledge-graph` which pulls in `agent-runtime` → cycle). The traits crate has only `serde_json`
and `zbot-stores-domain` as deps.

## Key Crate Details

### zbot-stores-domain
Domain value types that cross the persistence boundary. No methods that touch a DB.
Examples: `Goal`, `MemoryFact`, `Procedure`, `SessionEpisode`, `KgEpisode`, `WikiArticle`,
`UndistilledSession`, `DistillationStats`.

### zbot-stores-traits
Trait surfaces for each storage concern:
`ConversationStore`, `EpisodeStore`, `MemoryFactStore`, `KgEpisodeStore`, `CompactionStore`,
`OutboxStore`, `GoalStore`, `DistillationStore`, `RecallLogStore`, `WikiStore`, `ProcedureStore`.

### zbot-stores-sqlite
The production backend. It owns the full SQLite persistence surface:
- `DatabaseManager` — r2d2 connection pool, WAL mode, 8 connections
- `SqliteKgStore` — implements `KnowledgeGraphStore`
- `SqliteMemoryStore` / `GatewayMemoryFactStore` — implements `MemoryFactStore`
- `ConversationRepository`, `EpisodeRepository`, `MemoryRepository`, etc.
- `SqliteVecIndex` / `VectorIndex` — sqlite-vec based embedding index
- `KnowledgeDatabase` — knowledge-graph + embedding unified DB handle

### zbot-stores-conformance
Generic test functions parameterized over store traits. Impl crates call these from
integration tests to catch behavioral drift between backends.
