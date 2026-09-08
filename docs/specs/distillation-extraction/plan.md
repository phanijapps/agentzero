# Plan: Extract distillation into `services/distillation`

## Why

Slim the gateway-execution crate. Distillation (3,053 lines) + ward wiki
(583 lines) + ward artifact indexer (987 lines) = 4,623 lines that are
post-session subsystems, not execution orchestration.

## Move

```
FROM gateway-execution/src/          TO services/distillation/src/
  distillation.rs                     lib.rs (or split into focused files)
  ward_wiki.rs                        wiki.rs
  ward_artifact_indexer.rs            (stays in gateway — it indexes ward
                                       files, not distillation output)
```

## New crate: `services/distillation`

```
services/distillation/
  Cargo.toml
  src/
    lib.rs         — pub use surface
    distiller.rs    — SessionDistiller: orchestrator + phases
    extract.rs      — LLM prompt, response parsing, validation
    fact_store.rs   — supersede/dedup logic (stays here, not in adapter)
    graph.rs        — projection, governance, canonicalization
    strategy.rs     — failure clustering, strategy emergence
    wiki.rs         — ward wiki compilation
    transcript.rs   — build session transcript from messages
    provider.rs     — LLM client selection (target → default → first)
    types.rs        — DistillationResponse, ExtractedFact, etc.
```

## Dependencies

```toml
[dependencies]
agent-runtime = { path = "../../runtime/agent-runtime" }   # LlmClient, ChatMessage
gateway-events = { path = "../../gateway/gateway-events" } # EventBus
gateway-services = { path = "../../gateway/gateway-services" } # ProviderService, VaultPaths
zbot-stores = { path = "../../stores/zbot-stores" }       # MemoryFactStore, etc.
zbot-stores-traits = { path = "../../stores/zbot-stores-traits" } # ProcedureStore, etc.
zbot-stores-domain = { path = "../../stores/zbot-stores-domain" } # MemoryFact, Procedure
knowledge-graph = { path = "../knowledge-graph" }          # Entity, EntityType
api-logs = { path = "../api-logs" }                        # LogService
```

## What gateway-execution keeps

- Calls `SessionDistiller::distill(session_id, agent_id)` via trait or direct call
- Wires the stores into the distiller at startup
- Ward artifact indexer stays (it indexes files, not distillation output)

## Gateway-execution size change

| Before | After |
|---|---|
| ~48,000 lines | ~43,400 lines (distillation + wiki moved out) |

## Tasks

| Task | What |
|---|---|
| T1 | Create crate, move distillation.rs + ward_wiki.rs, fix imports |
| T2 | gateway-execution imports from the new crate, deletes old files |
| T3 | Split into focused files (distiller, extract, graph, strategy, etc) |
| T4 | Tests move with the code, verify green |
| T5 | Dead type audit, clippy, fmt |
