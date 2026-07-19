# zbot-stores-domain

Pure-data domain types for the AgentZero persistence layer. Only dependency is `serde`.

## Purpose

Home for every value type that crosses the persistence boundary. Types here:
- Have no methods that touch a database, file, or network
- Are needed by multiple crates (backend impls, gateway HTTP layer, agent tools)
- All backends serialize the same shape

## Modules & Key Types

| Module | Key Types |
|--------|-----------|
| `goal` | `Goal` — agent intent with status and progress |
| `memory_fact` | `MemoryFact`, `ScoredFact`, `StrategyFactInsert`, `StrategyFactMatch` |
| `procedure` | `Procedure`, `ProcedureSummary`, `PatternProcedureInsert` |
| `session_episode` | `SessionEpisode`, `ScoredEpisode`, `SuccessfulEpisode` |
| `kg_episode` | `KgEpisode`, `EpisodeSource` |
| `kg_ops` | `DecayCandidate`, `DuplicateCandidate`, `EntityNameEmbeddingHit`, `GraphView`, `RelationshipContext`, `StrategyCandidate` |
| `wiki` | `WikiArticle`, `WikiHit` |
| `distillation_ops` | `UndistilledSession`, `DistillationStats` |

## Intra-Repo Dependencies

None — this is a leaf crate (`serde` only).

## Notes

- HTTP request/response shapes live in the gateway HTTP layer (`gateway/src/http/`), not here.
- Repository structs and trait surfaces live in `zbot-stores-traits` and `zbot-stores-sqlite`.
- Add a new type here when multiple crates (backend + consumer) need to share it.
