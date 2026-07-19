# knowledge-graph

Entity type definitions, extraction logic, and name resolution for the AgentZero knowledge graph. This crate is **types + extraction only** — SQLite storage lives in `zbot-stores-sqlite::kg`.

## Build & Test

```bash
cargo test -p knowledge-graph    # 19 tests
```

## Modules

| File | Purpose |
|------|---------|
| `types.rs` | `Entity`, `Relationship`, `EntityType`, `RelationshipType`, `ExtractedKnowledge` |
| `extractor.rs` | `EntityExtractor` — heuristic + LLM-powered extraction |
| `resolver.rs` | `resolve()`, `normalize_name()`, `ResolveOutcome`, `MatchReason` |
| `error.rs` | `GraphError`, `GraphResult` |

## Key Types

| Type | Purpose |
|------|---------|
| `Entity` | Named entity with type, properties, timestamps, mention count |
| `Relationship` | Connection between two entities with confidence and context |
| `EntityType` | Person, Organization, Location, Concept, Tool, Project, Custom |
| `RelationshipType` | WorksFor, LocatedIn, RelatedTo, Created, Uses, PartOf, Mentions, Custom |
| `ExtractedKnowledge` | Batch result: `Vec<Entity>` + `Vec<Relationship>` |
| `ResolveOutcome` | Exact, Alias, Fuzzy, or None — used when upserting |

## Public API

```rust
pub use extractor::EntityExtractor;
pub use resolver::{normalize_name, resolve, MatchReason, ResolveOutcome};
pub use types::{Direction, Entity, EntityType, EntityWithConnections, ExtractedKnowledge,
    GraphStats, NeighborInfo, Relationship, RelationshipType, Subgraph};
```

## Where Storage Lives

SQLite persistence (`GraphStorage`, `KnowledgeDatabase`, traversal, causal) was relocated to `zbot-stores-sqlite::kg` during Slice D6b. Consumers should import via `zbot_stores_sqlite::SqliteKgStore` or the trait `zbot_stores::KnowledgeGraphStore`.

## Intra-Repo Dependencies

- None (this crate is a leaf — no internal crate dependencies)
