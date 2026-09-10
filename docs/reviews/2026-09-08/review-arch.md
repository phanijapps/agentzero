# Architecture Review — AgentZero (op_clean_crap)

Read-only review of module boundaries, layering, dependency direction, and cohesion.

## Summary

The workspace has **no dependency cycles** and the bottom layers (domain, traits, primitives) are clean. The three structural problems are:

1. **`VaultPaths` lives in `gateway-services` but is needed by the stores layer** — this single type causes every "stores → gateway" inversion in the graph.
2. **`zbot-stores-sqlite` (21,686 lines) has become a composition root** — it depends on `api-logs`, `execution-state`, `knowledge-graph` (services) and `gateway-services` (gateway) to wire itself, when composition belongs in the apps/gateway shell.
3. **`gateway-execution` (41,565 lines) and the gateway shell (31,859 lines) are both overloaded** — the "shell" contains 44 HTTP handler files, a 2,052-line AppState god object, and two subsystems (`a2a_tasks`, `durable_agent_tasks`) that aren't wiring.

Plus one half-finished migration: `zbot-stores` is now a 979-line re-export shim over `zbot-stores-traits` (63 files still import via the facade, 88 import directly).

## Dependency graph findings

Actual edges extracted from every `[dependencies]` section (dev-deps excluded). Verdict per suspicious edge:

### Stores layer depending upward

| Edge | Verdict | Why |
|---|---|---|
| `zbot-stores-sqlite → gateway-services` | **Wrong** | 21 uses of `VaultPaths`/`SharedVaultPaths` across 18 store files. Pure config type in the wrong home. |
| `zbot-runtime-sqlite → gateway-services` | **Wrong** | Same — 2 uses of `VaultPaths`. |
| `zbot-stores-sqlite → api-logs`, `→ execution-state` | **Wrong** | Concrete store crate wiring runtime services; composition belongs at the shell. |
| `zbot-runtime-sqlite → api-logs`, `→ execution-state` | **Wrong** | Same pattern. |
| `zbot-stores-sqlite → agent-runtime`, `→ agent-primitives` | **Questionable** | Only for `agent_runtime::llm::embedding::EmbeddingClient` (6 refs). An embedding trait belongs in `zbot-stores-traits` or `agent-primitives`. |
| `zbot-stores → knowledge-graph` | **Questionable** | `knowledge-graph` is labelled a service but is really domain vocabulary (Entity, EntityType, Relationship). Reclassify or fold types into `zbot-stores-domain`. |
| `zbot-stores-conformance → knowledge-graph` | **Keep (test harness)** | Acceptable for a test harness, resolves with the above. |
| `zbot-engram-adapter → agent-runtime` | **Questionable** | Single use of `EmbeddingClient`. Same fix as above. |
| `zbot-engram-adapter → knowledge-graph` | **Keep** | It maps engram entities to KG domain types — that's its job. |

### Services depending on gateway

| Edge | Verdict | Why |
|---|---|---|
| `distillation → gateway-events` | **Dead** | Declared in Cargo.toml, zero imports in src. Delete the dep. |
| `distillation → gateway-services` | **Wrong (but sanctioned-ish)** | Used for `VaultPaths` + `ProviderService` (LlmClient construction). Both should be injected: pass `Arc<dyn LlmClient>` or a factory, and fix VaultPaths' home. |
| `distillation → gateway-templates` | **Questionable** | One use: `Templates::get` for the wiki template. Move the template constant into distillation or pass text in. |

### Other notable edges

| Edge | Verdict | Why |
|---|---|---|
| `gateway-execution → zbot-runtime-sqlite` | **Questionable** | 20 prod-code uses of `DatabaseManager` (conversations.db pool). A concrete SQLite type woven through the execution layer; should be a trait seam (`[dev-dependencies]` has stores-sqlite correctly). |
| `gateway-bridge → zbot-runtime-sqlite` | **Questionable** | 3 uses, includes two `.unwrap()` on `DatabaseManager::new` in prod paths. |
| `gateway-execution → distillation` | **Keep** | Correct direction after the extraction (gateway consumes services). |
| `agent-runtime → zbot-stores-traits`, `agent-tools → zbot-stores-traits` | **Keep** | Deliberate (documented in traits crate), but **AGENTS.md dependency diagram omits it** — doc gap. |
| `gateway-services → discovery`, `→ gateway-memory` | **Keep** | gateway-services is genuinely a mid-layer service hub; fine. |
| `apps/* → gateway` etc. | **Keep** | Correct. |

## Boundary violations (duplicate / leaky abstractions)

1. **`zbot-stores` vs `zbot-stores-traits` split-brain.** `zbot-stores` is now a facade: every module is `pub use zbot_stores_traits::{...}` (979 lines total, 3 real trait files remain: `KnowledgeGraphStore` + types). 63 files import `zbot_stores::X`, 88 import `zbot_stores_traits::X`. The migration stopped halfway. Finish it: move `KnowledgeGraphStore` into `zbot-stores-traits` (folding the needed knowledge-graph types), delete the facade, or make `zbot-stores` contain *only* the KG trait and rename it.

2. **Two event vocabularies joined by hand-rolled converters.** `gateway-events::GatewayEvent` (wire/UI events) vs engine stream events in `agent-runtime`, converted ad-hoc in `gateway-execution/src/events.rs::convert_stream_event` (and again in `stream_event_processor.rs`, 972 lines). This is conversion logic duplicated across layers. One canonical mapping module would do.

3. **Leak check (stores traits vs engram adapter): clean.** `gateway-execution` production code uses trait-level stores; its `zbot-stores-sqlite`/`zbot-engram-adapter` imports are all `#[cfg(test)]`-gated (verified). The engram adapter exposes governance/mapping internally but the trait surface holds. No action.

4. **`knowledge-graph` is domain vocabulary living in services/.** Its types (`Entity`, `EntityType`, `Relationship`, `Direction`) are used by stores traits, sqlite impl, engram adapter, distillation, gateway-memory. It has zero upward deps — it behaves like `zbot-stores-domain`. Either move it under `stores/` or accept it as a shared-types crate and document it as such in AGENTS.md.

## Cohesion findings (modules in the wrong home)

All inside `gateway-execution` (module → size → verdict):

| Module | Lines | Verdict |
|---|---|---|
| `delegation/` | 3,812 | **Keep** — core execution orchestration. |
| `sleep/handoff_writer` | 1,302 | **Move to `gateway-memory`** — it writes memory facts; the in-file comment arguing it "bakes in prompt conventions" describes a parameter, not a home. |
| `peer_messaging.rs` | 1,139 | **Extract** — durable peer messaging is an independent subsystem (own module or services crate); execution only needs to *call* it. |
| `ingest/` | 1,121 | **Keep** — execution pipeline. |
| `middleware/` (intent, resource_index, ward_scaffold) | 1,096 | **Keep** — genuinely execution middleware. |
| `ward_artifact_indexer.rs` | 987 | **Move to `services/`** — zero-LLM post-session file indexer; depends only on stores traits + knowledge-graph. Same lifecycle as distillation. Its own doc header says "backend-agnostic, zero changes needed." |
| `artifacts/`, `archiver.rs`, `indexer/`, `curator` | ~2,200 | **Keep** — session lifecycle. |
| `session_title.rs` | 129 | **Move** (minor) — LLM summarization; distillation-adjacent. |
| `a2a/` | 157 | **Fold into `gateway-a2a`** — there's already a dedicated crate; this is a split-brain (execution-boundary types for remote A2A work live in execution). |

Gateway **shell** (31,859 lines — not a shell):

| Item | Lines | Verdict |
|---|---|---|
| `state/mod.rs` (`AppState`, 50 public fields, 3 constructors) | 2,052 | **Decompose** — god object; split into capability groups or builder-per-domain. |
| `http/commissioning.rs` | 2,121 | **Keep in shell, consider splitting file.** |
| `a2a_tasks.rs` + `durable_agent_tasks.rs` | 2,809 | **Extract** — subsystems, not wiring. |
| `websocket/` | ~3,000 | **Extract to `gateway-ws`** — `gateway-ws-protocol` holds types only; handlers/subscriptions live in the shell. |
| `cron/mod.rs` | 1,069 | **Fold into `gateway-cron`** — the dedicated crate exists. |

## Crate size distribution

| Crate | LOC | Note |
|---|---|---|
| gateway-execution | 41,565 | Still the god crate; biggest file `delegation/spawn.rs` 2,732 |
| gateway shell | 31,859 | Should be <10k for a wiring layer |
| agent-runtime | 27,956 | Acceptable (engine + rig adapter) |
| zbot-stores-sqlite | 21,686 | Impl + composition mixed; kg/ subtree alone is large |
| gateway-memory | 19,423 | Acceptable for its scope |
| gateway-services | 18,284 | Grew into a hub; `VaultPaths` here causes the inversions |
| zbot-engram-adapter | 16,881 | Acceptable |
| agent-tools | 14,623 | Fine |
| execution-state | 10,439 | `repository.rs` 4,156 — largest single file in services; candidate split |
| everything else | <4,300 | Healthy |

## Proposed target layout

```
stores/
  zbot-stores-domain      (absorbs or aliases knowledge-graph types)
  zbot-stores-traits      (ALL store traits incl. KnowledgeGraphStore + EmbeddingClient trait)
  zbot-stores-sqlite      (impl only; no gateway/services deps)
  zbot-engram-adapter
  zbot-conversation
services/
  knowledge-graph         (either stays as shared-vocab crate, documented, or types move to domain)
  distillation            (LlmClient injected; no gateway deps)
  ward-index              (from gateway-execution::ward_artifact_indexer)
gateway/
  gateway-events, gateway-ws (+ handlers from shell), gateway-a2a (+ execution::a2a)
  gateway-cron (+ shell cron)
  gateway-services        (VaultPaths moved OUT to agent-primitives or a paths crate)
  gateway-execution       (~34k after moves)
  gateway                 (true shell: server + http + slim AppState)
apps/
  daemon, cli, ui
```

## Migration order (cheapest first)

1. **Delete dead dep** `distillation → gateway-events` (one Cargo.toml line).
2. **Finish the traits migration** — rewrite the 63 facade imports to `zbot_stores_traits::`, then decide `zbot-stores`' fate (delete or KG-trait-only).
3. **Move `VaultPaths`** out of `gateway-services` (to `agent-primitives` or a new `zbot-paths`); fix the ~25 store-side imports; delete gateway-services from both store crates' Cargo.toml.
4. **Move `ward_artifact_indexer` → `services/ward-index`**; move `sleep/handoff_writer` → `gateway-memory`; fold `execution::a2a` into `gateway-a2a` (all three are self-contained, trait-level deps).
5. **Untangle `zbot-stores-sqlite` composition** — move bootstrap/wiring into the gateway shell; store impl keeps only SQLite code.
6. **Extract `gateway-ws` (handlers) and fold shell cron into `gateway-cron`**; split `AppState` into capability structs.
7. **Inject `LlmClient` into distillation** (drop `gateway-services`/`gateway-templates` deps); replace `DatabaseManager` in gateway-execution with a trait seam.
8. **Docs**: update AGENTS.md dependency diagram to include `agent-runtime → zbot-stores-traits` and the `knowledge-graph` classification decision.

## Bonus observations (DRY/KISS)

- `gateway-bridge/src/provider.rs:206` and `plugin_manager.rs:396,410`: `DatabaseManager::new(paths).unwrap()` in prod code — three unwraps on DB construction.
- `invoke/builder_tests.rs` (2,518 lines) lives beside `builder.rs` (1,345) — fine, but the builder itself is a mega-constructor; the ExecCtx consolidation could continue into it.
- AGENTS.md says `services/*` depend on "stores + runtime" — reality (distillation) also needs the two sanctioned gateway crates today; after step 7 it won't.
