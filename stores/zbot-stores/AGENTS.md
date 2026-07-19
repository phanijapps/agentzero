# zbot-stores

Backend-agnostic persistence interfaces for AgentZero. Defines store traits and types — no database drivers.

## Purpose

Public design surface for the persistence layer. Consumers depend on `zbot-stores` to get traits and types without coupling to a specific database backend. The concrete implementation is in `zbot-stores-sqlite`.

## Modules

| Module | Purpose |
|--------|---------|
| `knowledge_graph` | `KnowledgeGraphStore` trait + query types |
| `memory_facts` | `MemoryFactStore` trait + metric types |
| `types` | Shared types: `EntityId`, `Direction`, `ResolveOutcome` |
| `extracted` | `ExtractedKnowledge` — batch extraction result |
| `error` | `StoreError`, `StoreResult` |

## Key Exports

```rust
pub use knowledge_graph::{KnowledgeGraphStore, GraphView, DecayCandidate, ...};
pub use memory_facts::{MemoryFactStore, MemoryAggregateStats, MemoryHealthMetrics, ...};
pub use types::*;
pub use extracted::ExtractedKnowledge;
```

All trait surfaces from `zbot-stores-traits` are also re-exported here for the canonical import path.

## Intra-Repo Dependencies

- `zbot-stores-domain` — domain value types
- `zbot-stores-traits` — trait surfaces (re-exported)
- `knowledge-graph` — `Entity`, `Relationship`, `EntityType` etc. from the types module

## Notes

- Never import `zbot-stores-sqlite` directly from business logic — always go through `zbot-stores` traits.
- The gateway wires the concrete `SqliteKgStore` / `GatewayMemoryFactStore` via `persistence_factory.rs`.
