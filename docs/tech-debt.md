# Persistence Layer Tech Debt — Registry & Fix Plan

> **Note (2026-05-03+):** The SurrealDB backend referenced as a future target throughout this doc was implemented in 2026-04-28 and reverted in commit `d823b7bd: chore: remove SurrealDB backend, keep clean trait-routed SQLite`. The trait abstraction work captured below (TD-010, TD-011, TD-012, TD-014, TD-021, TD-022, TD-023, TD-030–033, TD-042) is still load-bearing for the SQLite impl — only the SurrealDB-specific framing is now historical. The phased fix plan that mentions SurrealDB as the destination should be read as "the abstraction shipped; the SurrealDB sidecar did not."

## Why this doc exists

The medium-term plan is to replace `knowledge.db` (currently SQLite + `sqlite-vec` + FTS5) with **SurrealDB** (memory + knowledge graph in one embedded store). `conversations.db` will stay on SQLite indefinitely.

This doc tracks every piece of tech debt that stands between "today" and "switch-flip ready" — the state where adding SurrealDB is a single new crate plus a config switch, with no edits required in `gateway-execution/`, `services/*`, or business logic.

It is also the canonical home for persistence-layer tech-debt items that aren't migration-related. Treat it as a **living document**: check items off as they ship, add new items when discovered.

## How to use this doc

- **Inventory section** — flat list of every debt item, grouped by category. Each item has a stable ID (`TD-NNN`), severity, file:line refs, why it's debt, and a concrete fix approach.
- **Phased fix plan** — sequenced phases, each closing one or more inventory items.
- **Status field** on each item: `pending` / `in-progress` / `done`. Update as work happens.
- **PR linking** — when an item ships, replace `pending` with `done — PR #N`.
- **Adding items** — see the "How to add an item" section at the bottom.

## Severity & scope conventions

| Symbol | Meaning |
|---|---|
| 🔴 Critical | Real bug, or hard blocker for SurrealDB readiness |
| 🟠 High | Significant abstraction debt; on the critical path |
| 🟡 Medium | Mechanical cleanup; easy win |
| 🟢 Low | Stylistic; do alongside related work, not on its own |

Scope tags:
- **[K]** = knowledge-side (memory + KG; will move to SurrealDB)
- **[C]** = conversations-side (stays SQLite forever)
- **[B]** = bug, independent of migration
- **[Both]** = touches both halves

---

## Inventory

### Real bugs

#### TD-001 ✅ [B] `archiver.rs` archives now wrapped in a transaction
- **Location:** `gateway/gateway-execution/src/archiver.rs:186-210`
- **Resolution:** Wrapped the three writes (`DELETE FROM messages`, `DELETE FROM execution_logs`, `UPDATE sessions SET archived = 1`) in a single `with_connection` block bounded by `conn.unchecked_transaction()` and `tx.commit()`. Added regression test `archive_session_atomically_deletes_and_marks` that seeds a session with messages + logs and verifies all three writes succeed end-to-end.
- **Status:** done — PR #73 (commit `dbcebea`)

### Abstraction-shape debt — knowledge side (critical path for SurrealDB)

#### TD-010 ✅ [K] `KnowledgeGraphStore` trait extracted (Phase 1 done)
- **Location:** `services/knowledge-graph/src/storage.rs` (concrete `GraphStorage` retained as the SQLite impl backing).
- **Resolution:** `KnowledgeGraphStore` trait now lives in new `zbot-stores` interface crate with all 14 methods (entity CRUD, aliases & resolution, relationships, bulk ingest, read paths, maintenance). New `zbot-stores-sqlite` adapter crate implements the trait by wrapping `Arc<GraphStorage>` and bridging sync rusqlite → async via `spawn_blocking`. New `zbot-stores-conformance` crate holds cross-impl scenarios (one scaffold scenario; more added incrementally). `AppState` exposes `kg_store: Option<Arc<dyn KnowledgeGraphStore>>` alongside the existing `graph_service`. One HTTP handler (`search_entities`) migrated as proof of pattern. Three atomicity bugs found and fixed in `GraphStorage` along the way: `delete_entity_by_id` (commit 324b573), `store_knowledge` (commit cc59cde) — both wrapped in `unchecked_transaction()` to honor the trait contract.
- **Status:** done — Phase 1 implementation plan executed across 14 tasks on `feature/phase1-kg-store-extraction`

#### TD-011 ✅ [K] `CausalEdgeStore` rusqlite leak removed
- **Location:** `services/knowledge-graph/src/causal.rs` — `pub fn new(db: Arc<KnowledgeDatabase>)` (was: `Arc<Mutex<rusqlite::Connection>>`).
- **Resolution:** Constructor now takes `Arc<KnowledgeDatabase>`. Struct field changed from `Arc<Mutex<rusqlite::Connection>>` to `Arc<KnowledgeDatabase>`. Method bodies use `self.db.with_connection(|conn| ...)` instead of `self.conn.lock()`. All 4 existing causal-edge tests pass; no behavioural change. No public callers exist outside the test module — `CausalEdgeStore` is constructed only in tests today, so updating callers reduces to fixing the test fixture.
- **Status:** done — commit `93a75bd`

#### TD-012 🟠 [K] `gateway-execution/sleep/*` bypasses persistence abstractions
- **Locations:** All in `gateway/gateway-execution/src/sleep/`
  - `synthesizer.rs` (~12 stmts; `kg_*`, `memory_facts`, `kg_episodes`, `session_episodes`)
  - `kg_backfill.rs` (~11 stmts; `kg_entities`, `kg_relationships`, `kg_compactions`)
  - `embedding_reindex.rs` (~9 stmts; `vec0` reindex pipeline)
  - `orphan_archiver.rs` (~8 stmts; `kg_*` cleanup)
  - `pattern_extractor.rs` (~9 stmts; cross-DB read)
- **Why debt:** Roughly 60 statements touching knowledge-side tables directly via raw `rusqlite::Connection`. None of these would survive a SurrealDB swap unless they all route through `KnowledgeGraphStore` and `MemoryFactStore`.
- **Fix:** After TD-010 lands, route each file's reads/writes through the appropriate store trait. Reindex is the trickiest — `vec0`-specific schema rebuild becomes a SurrealDB-specific schema rebuild — so hide it behind `KnowledgeGraphStore::reindex_embeddings(new_dim)` (or equivalent) so each impl owns its physical layout.
- **Per-file decisions:**
  - `orphan_archiver.rs` ✅ — read path migrated (Phase 3a, PR #76); write path migrated (Phase 3b, PR #77). Together: file uses no raw rusqlite in production logic.
  - `kg_backfill.rs` ⏸ **intentionally NOT migrated** — one-shot bootstrap that runs once at daemon startup to retrofit pre-existing rows with new schema fields. Forcing it through the trait would mean adding ~6 new methods (list-with-missing-property, batch-update-properties, compaction marker check/set) just for this single bootstrap concern. When SurrealDB launches, it'll start with the new schema and won't need the same backfill — each impl handles its own schema-evolution story. Marking as decision-explicit rather than pending. (Phase 3c)
  - `embedding_reindex.rs` ✅ — Phase 3d. Orchestration moved to `stores/zbot-stores-sqlite/src/reindex.rs`; `SqliteKgStore::reindex_embeddings` now drives it via a new `with_embedding_client` constructor. The gateway-side `sleep/embedding_reindex.rs` stays as a thin re-export wrapper so the two existing progress-aware callers (`state::reconcile_embeddings_at_boot` and the `/api/embeddings/configure` SSE handler) keep emitting per-table `Health::Reindexing` events to the UI's `EmbeddingProgressModal` — the trait surface itself is intentionally fire-and-report (no progress callback) so it stays portable across SQLite / SurrealDB.
  - `synthesizer.rs` 🛑 still deferred — `MemoryFactStore` relocation is now DONE (trait lives in `stores/zbot-stores-traits`, re-exported from `stores/zbot-stores`). The remaining blocker is new trait methods (multi-session aggregation, cosine dedup over vec0). Belongs in Phase 5+.
  - `pattern_extractor.rs` 🛑 still deferred — `MemoryFactStore` relocation unblocks the memory-side ops, but cross-DB read (knowledge.db + conversations.db) plus LLM orchestration still needs `ProcedureStore` + `EpisodeStore` traits, not part of Phase 3 scope. Phase 5+.
- **Status:** in progress — orphan_archiver complete (Phase 3a/3b); embedding_reindex done (Phase 3d); kg_backfill explicitly opted out (3c); synthesizer + pattern_extractor still deferred (MemoryFactStore relocation now done; remaining blockers are new trait methods + ProcedureStore/EpisodeStore traits)

#### TD-013 ✅ [K] `VectorIndex` folded into store traits
- **Location:** `gateway/gateway-database/src/vector_index.rs:15-32`
- **Resolution:** Decided in `docs/architecture/future-state/persistence-readiness-design.md` (Section: Trait surface). **No public `VectorIndex` trait** in `zbot-stores`. Vector ops are part of `MemoryFactStore::recall` and `KnowledgeGraphStore::resolve_entity`. SQLite impl keeps `SqliteVecIndex` internally as an implementation detail; SurrealDB impl uses HNSW indexes inline on records.
- **Status:** resolved by design — fix lands as part of TD-010 / TD-012 implementation

#### TD-014 🟠 [K] Knowledge-side ops in `services/execution-state/repository.rs` co-mingle with conversations ops
- **Location:** `services/execution-state/src/repository.rs` (entire file — ~99 fns, ~81 stmts). Touches `sessions`, `agent_executions`, `messages` (conversations.db) **and** `memory_facts`, `kg_relationships`, `kg_entities`, `recall_log` (knowledge.db). Note also the developer-acknowledged pain at line 426: `"the with_connection trait hands us &Connection, not &mut"`.
- **Why debt:** A single repo straddles the two databases that are about to diverge. Once knowledge.db moves to SurrealDB, every method in this file has to know which backend to talk to — turning the repo into a manual coordinator.
- **Fix:** Split into a leaner `ExecutionStateStore` (sessions/executions/messages — stays SQLite) and remove all direct knowledge-side table access from this file, replacing with calls into `KnowledgeGraphStore` + `MemoryFactStore`.
- **Status:** unblocked (MemoryFactStore relocated to `stores/zbot-stores-traits` / re-exported from `stores/zbot-stores`); migration of `repository.rs` knowledge-side ops still pending as a separate multi-PR workstream
- **Phase 6 deferral note:** Multi-PR workstream — 31 SQL callsites in `services/execution-state/src/repository.rs` (2780 lines). Each migrating function needs careful per-method scrutiny. NOT in scope for the Phase 6 hygiene PR; tracked as the next standalone workstream.

### Abstraction-shape debt — conversations side (NOT critical for migration)

#### TD-020 🟡 [C] `DbProvider` / `StateDbProvider` traits are SQLite-shaped
- **Locations:**
  - `services/api-logs/src/repository.rs:17-21` — `DbProvider`
  - `services/execution-state/src/repository.rs:14-17` — `StateDbProvider`
- **What:** Both define `fn with_connection<F, R>(&self, f: F) -> Result<R, String> where F: FnOnce(&Connection) -> Result<R, rusqlite::Error>`. The closure parameter is a raw rusqlite `&Connection`; the inner error type is `rusqlite::Error`. The trait *is* SQLite.
- **Why debt:** Even though conversations stays SQLite, this shape blocks any cross-cutting work (observability decorators, swapping pool implementations, in-memory test doubles, etc).
- **Fix:** Reshape to method-per-operation traits — `LogStore::append`, `LogStore::query_by_session`, `ExecutionStateStore::insert_message`, etc. Internal pool can stay rusqlite; the contract stops leaking.
- **Status:** pending (does not block SurrealDB switch)
- **Phase 6 deferral note:** Wide-reach trait reshape — `DbProvider` / `StateDbProvider` are referenced from many service crates. Reshape requires per-method-per-trait migration. Low priority since conversations stays SQLite forever; trait shape staying SQLite-flavored is acceptable. Not bundled into the TD-021 / TD-022 hygiene PR because the work touches a different blast radius.

#### TD-021 ✅ [C] `ConversationStore` trait extracted (Phase 6 hygiene)
- **Location:** `gateway/gateway-database/src/repository.rs` — `ConversationRepository` now implements `zbot_stores_traits::ConversationStore`.
- **Resolution:** Hygiene scaffold landed alongside TD-022. New `stores/zbot-stores-traits/src/conversation.rs` defines `ConversationStore` with two trait-friendly methods (`get_session_ward_id`, `get_session_agent_id`) that mirror `ConversationRepository` verbatim. The richer methods returning `Message` / `Vec<Message>` / `agent_runtime::ChatMessage` stay on the concrete type — hoisting `Message` out of `gateway-database` into `zbot-stores-traits` is deferred until a consumer actually wants trait-erased reads. The point of this scaffold is symmetry with `KnowledgeGraphStore` and `OutboxStore`, not full method coverage.
- **Consumer migration:** explicitly NOT part of this scaffold. All sites continue to hold `Arc<ConversationRepository>` directly; promoting to `Arc<dyn ConversationStore>` is a separate workstream tracked under TD-023's retirement halves.
- **Status:** done — Phase 6 hygiene

#### TD-022 ✅ [C] `OutboxStore` trait extracted (Phase 6 hygiene)
- **Location:** `gateway/gateway-bridge/src/outbox.rs` — `OutboxRepository` now implements `zbot_stores_traits::OutboxStore`.
- **Resolution:** New `stores/zbot-stores-traits/src/outbox.rs` defines `OutboxStore` with the lifecycle core (`insert_item`, `mark_inflight`, `mark_sent`, `reset_inflight`). The impl on `OutboxRepository` forwards to the existing inherent methods, mapping `BridgeError` → `String` at the trait boundary (the trait can't pull in `BridgeError` without a circular crate dep). Deliberately omitted from the trait surface: `mark_failed` (carries `chrono::DateTime<Utc>`), `get_unacked` / `get_since` / `get_retryable` (return `Vec<OutboxItem>` whose row type lives in `gateway-bridge`), `reset_all_inflight`, `cleanup_sent` — these stay on the concrete type until a cross-cutting consumer needs them in the trait.
- **Consumer migration:** explicitly NOT part of this scaffold. All sites continue to hold `Arc<OutboxRepository>` directly; promoting to `Arc<dyn OutboxStore>` is out of scope.
- **Status:** done — Phase 6 hygiene

#### TD-023 ⏳ [Both] `AppState` factory pattern established; `graph_service` retirement mostly done
- **Location:** `gateway/src/state/mod.rs` — concrete persistence types alongside trait objects.
- **Progress (Phase 5):** Factory pattern landed at `gateway/src/state/persistence_factory.rs`. `kg_store` construction goes through `build_kg_store_from_storage()`; the canonical `build_kg_store(knowledge_db, embedding_client)` entrypoint is gated until `graph_service` retires (it's where the SurrealDB config-driven branch will live). When SurrealDB support lands, this is where the branch goes — consumers don't need to change because they hold `Arc<dyn KnowledgeGraphStore>`.
- **Progress (MemoryFactStore relocation):** Trait lives in the dependency-light `stores/zbot-stores-traits` crate (re-exported from `stores/zbot-stores` for the design-canonical `zbot_stores::MemoryFactStore` import path). `SqliteMemoryStore` now lives at `stores/zbot-stores-sqlite/src/memory_facts.rs` (a re-export of `gateway_database::GatewayMemoryFactStore`). New `persistence_factory::build_memory_store(memory_repo, embedding_client)` factory; new `AppState::memory_store: Option<Arc<dyn MemoryFactStore>>` field coexists with the legacy concrete `memory_repo` field (same pattern as `kg_store` / `graph_service`).
- **Progress (HTTP handler migration):** All `state.graph_service` callsites in `gateway/src/http/graph.rs` (8 handlers — `get_graph_stats`, `list_entities`, `list_relationships`, `get_entity_neighbors`, `get_entity_subgraph`, `search_entities`, `graph_stats`, `all_entities`, `all_relationships`) routed through `state.kg_store`. `KnowledgeGraphStore` grew 10 new trait methods (`graph_stats`, `list_entities`, `list_relationships`, `get_neighbors_full`, `get_subgraph`, `count_all_entities`/`relationships`, `list_all_entities`/`relationships`, `vec_index_health`). `MemoryFactStore` grew 3 (`aggregate_stats`, `health_metrics`, `count_all_facts`). `gateway/src/http/embeddings.rs::get_health` migrated to `kg_store.vec_index_health()`. `gateway/src/http/memory.rs::stats` + `health` migrated to trait stores. `gateway/src/http/memory_search.rs` episode LIKE search relocated to `EpisodeRepository::keyword_search` (typed-record level). Response shapes unchanged across the board.
- **Deferred (HTTP-handler retirement follow-up):**
  - `gateway/src/http/memory.rs` typed-MemoryFact CRUD handlers (`list_memory_facts`, `search_memory_facts`, `get_memory_fact`, `delete_memory_fact`, `create_memory_fact`, `search_all_memory_facts`, `list_all_memory_facts`) still call `state.memory_repo` directly. The `MemoryFactStore` trait surface returns `serde_json::Value`; migrating these handlers requires hoisting `MemoryFact` from `gateway-database` up to `zbot-stores`, which has 11 import sites (intentionally a separate workstream).
  - `gateway/src/http/ward_content.rs` — both handlers stay on `state.memory_repo` plus on-demand `WardWikiRepository`/`ProcedureRepository`/`EpisodeRepository` constructors. Same blocker as above plus separate trait-abstraction work for the wiki/procedure/episode subsystems.
  - `gateway/src/http/graph.rs::reindex_all_wards` still reaches into `state.graph_service` + `state.kg_episode_repo` because `gateway_execution::ward_artifact_indexer::index_ward_with_options` accepts `&Arc<GraphStorage>` + `&KgEpisodeRepository`; plumbing trait objects through `gateway-execution` is its own change.
  - `gateway/src/http/embeddings.rs::configure` (SSE reindex) still forwards `state.knowledge_db` to `gateway_execution::sleep::embedding_reindex::reindex_all` because the trait's `reindex_embeddings` method intentionally has no progress callback (different impls rebuild differently; trait surface stays portable). Adding a streaming variant is a separate workstream.
  - Concrete `state.graph_service` / `state.memory_repo` / `state.knowledge_db` fields stay on `AppState` for now — they have remaining consumers in sleep jobs, distillation, default-policy seeding, etc. Retirement of those fields is a separate cleanup PR after the remaining consumer migrations.
- **Status:** factory pattern done; HTTP-handler migration mostly done (graph + embeddings + stats/health); typed-record CRUD handlers and AppState field retirement remain

### Dialect-portability debt

#### TD-030 ✅ [K] `datetime('now')` knowledge-side callsites replaced with Rust timestamps
- **Locations cleared (Phase 2):**
  - `gateway/gateway-database/src/memory_repository.rs:473, 628, 667, 1103, 1156, 1172, 1202` — 7 callsites (commit `e0d89fe`)
  - `gateway/gateway-database/src/procedure_repository.rs:247, 258, 272` — 3 callsites (commit `67a6dd2`)
  - `gateway/gateway-database/src/episode_repository.rs:235` — 1 relative-time callsite using `chrono::Duration::days(14)` (commit `29a1b02`)
- **Resolution:** All knowledge-side runtime SQL callsites now bind a `chrono::Utc::now().to_rfc3339()` parameter instead of using SQLite's `datetime('now')`. Relative-time math (e.g., `'-14 days'`) is computed via `chrono::Duration::days(14)` and bound as a parameter. The SQLite impl behaves identically; the contract no longer assumes the DB has a clock — needed for the future SurrealDB swap.
- **Out of scope (deliberately retained):** `knowledge_schema.rs:31, 736` (schema bootstrap upsert + test fixture — impl-internal schema territory; addressed in TD-032 if/when revisited). Conversations-side callsites (`schema.rs`, `repository.rs`, `outbox.rs`, `archiver.rs`) — conversations stays SQLite forever per the design, so portability concerns don't apply.
- **Follow-up tracked separately:** TD-042 captures the `julianday('now')` callsite at `memory_repository.rs:629` (semantic change required, not a mechanical substitution).
- **Status:** done — Phase 2

#### TD-031 ✅ [K] `INSERT OR REPLACE` / `INSERT OR IGNORE` semantics already at trait level (closed by Phase 1)
- **Locations audited (Phase 2):** All `INSERT OR REPLACE` / `INSERT OR IGNORE` callsites in the workspace are inside impl crates (`gateway-database`, `services/knowledge-graph`, `services/execution-state`). Specifically: `recall_log_repository.rs`, `schema.rs`, `knowledge_schema.rs`, `memory_repository.rs:1102` (embedding_cache), `causal.rs`, `kg_episode_repository.rs` (×3), `storage.rs` (×3 alias inserts), `execution-state/repository.rs:534` (temp-table trick).
- **Resolution:** Phase 1's trait surface (`KnowledgeGraphStore`, `MemoryFactStore` in `stores/zbot-stores`) already exposes upsert vocabulary — `upsert_entity`, `upsert_relationship`, `add_alias`, `save_fact`, etc. The SQLite-specific `INSERT OR …` syntax lives entirely inside the SQLite impl crate and is not visible to any consumer crate. The future SurrealDB impl will use SurrealDB's record-upsert semantics in its own crate — no contract change needed.
- **Status:** done by Phase 1 — audit confirmed in Phase 2 (no code changes required)

#### TD-032 ✅ [K] Schema bootstrap is per-impl, idempotent, no cross-version migration in scope
- **Locations:** `gateway/gateway-database/migrations/{v23_wiki_fts.sql, v24_global_scope_backfill.sql}`, plus inline schema strings in `gateway/gateway-database/src/schema.rs` and `knowledge_schema.rs`.
- **Resolution direction (per design doc):** Each impl crate has a private `bootstrap_schema()` function called once at startup. SQLite impl runs idempotent `CREATE TABLE IF NOT EXISTS …`. SurrealDB impl runs idempotent `DEFINE TABLE … IF NOT EXISTS`. **No `Migrator` trait in `zbot-stores`.** Cross-version data migration and SQLite→SurrealDB data migration are explicitly out of this design's scope — those become a future `zbot-stores-migrate` crate when actually needed.
- **Phase 4 outcome:** Added `stores/zbot-stores-sqlite/src/bootstrap.rs` with `bootstrap_schema()` as the canonical hook point for the SQLite backend. Today it delegates to `gateway-database`'s `KnowledgeDatabase::new` (which runs the bootstrap as a constructor side effect) — pattern established without churning 1000+ lines of schema DDL. (Historical: a `stores/zbot-stores-surreal/src/bootstrap.rs` mirror was implemented in 2026-04-28 and reverted in commit `d823b7bd`; that crate no longer exists. The SQLite-side bootstrap pattern stays load-bearing.) Full DDL relocation deferred until shapes need to diverge between backends.
- **Status:** ✅ done (Phase 4) — pattern established, full schema relocation deferred until proven necessary

#### TD-033 ✅ [Both] Hardcoded table-name string literals removed from HTTP layer
- **Locations historically tracked:** ~90 in `services/knowledge-graph/src/storage.rs`, ~40 in `services/execution-state/src/repository.rs`, ~12 in `gateway/gateway-bridge/src/outbox.rs`. Plus inline string literals (e.g. `"memory_facts"`, `"kg_episodes"`, `"session_episodes"`) scattered across `gateway/src/http/memory.rs`, `gateway/src/http/embeddings.rs`, `gateway/src/http/memory_search.rs`.
- **Resolution:** As of the TD-023 HTTP handler migration, no `gateway/src/http/*.rs` file references table-name string literals. The remaining literals live inside impl crates (`gateway-database`, `services/knowledge-graph`, `services/execution-state`, `gateway-bridge`) where they belong — that's how impl-internal SQL is supposed to be structured. The HTTP layer goes through trait methods that hide the table names.
- **Status:** ✅ closed naturally — handlers no longer carry impl-level coupling

### Code smell (low priority)

#### TD-040 🟢 [K] Dynamic SQL via `format!()` in `embedding_reindex.rs`
- **Location (historical):** `gateway/gateway-execution/src/sleep/embedding_reindex.rs:127-218` — five `format!()` SQL builders interpolating fields of `ReindexTarget`.
- **Current location:** `stores/zbot-stores-sqlite/src/reindex.rs` — same code, same const-driven safety, just lives in the right crate now.
- **What:** **Not** a SQL injection — `REINDEX_TARGETS` is a `&'static [ReindexTarget]` const; all interpolated fields are `&'static str` baked into the binary. Stylistic only.
- **Status:** ✅ moved with TD-012 Phase 3d. Orchestration relocated to `zbot-stores-sqlite` where it belongs (SQLite-impl-internal).

#### TD-041 🟢 [Both] Mixed parameter binding styles
- **What:** `?`, `?1`/`?2`/…, and named `:foo` params are all in use across the codebase.
- **Fix:** No urgency. Standardize on numbered (`?1`) opportunistically as files are touched for other work.
- **Status:** opportunistic

#### TD-042 ✅ [K] `julianday('now')` date-arithmetic at memory_repository.rs:629
- **Location:** `gateway/gateway-database/src/memory_repository.rs:629` — `julianday('now') - julianday(updated_at) > ?2` (in `decay_stale_facts`).
- **Why debt:** `julianday()` is a SQLite-specific time function. Like `datetime('now')`, it bakes a clock assumption into the SQL — but unlike `datetime('now')`, it can't be replaced by a single bound parameter. The fix requires a semantic change: pre-compute a threshold timestamp in Rust (`Utc::now() - Duration::days(N)`) and rewrite the WHERE clause to compare `updated_at < ?threshold`.
- **Phase 4 fix:** `decay_stale_facts` now computes `cutoff = (Utc::now() - chrono::Duration::days(older_than_days as i64)).to_rfc3339()` in Rust and binds it as `?3` in `WHERE updated_at < ?3`. Params reordered: `?1` decay_factor, `?2` now (for `updated_at` write), `?3` cutoff. Same flavor as Phase 2's `datetime('now')` cleanup — keeps the SQL portable across SQLite and SurrealDB.
- **Status:** ✅ done (Phase 4)

### Non-functional / UI build hygiene

#### TD-043 🟡 [B] 89 pre-existing TypeScript errors in UI test files block `npm run build`
- **Location:** `apps/ui/src/` — 11 affected `*.test.ts(x)` files:
  - `App.extra.test.tsx`
  - `features/logs/log-hooks-extended.test.ts`
  - `features/logs/useSessionTrace.test.ts`
  - `features/memory/command-deck/SearchResults.test.tsx`
  - `features/setup/SetupWizard.test.tsx`
  - `features/setup/steps/AgentsStep.test.tsx`
  - `features/setup/steps/ProvidersStep.test.tsx`
  - `features/setup/steps/ReviewStep.test.tsx`
  - `features/setup/steps/SkillsStep.test.tsx`
  - `services/transport/http.embeddings.test.ts`
  - `services/transport/http.ws.test.ts`
- **Why debt:** `tsc` type-checks all of `src/` (including tests) before `vite build`. 89 errors in test fixtures stop the build dead. Production code itself has **zero** TS errors — verified by counting errors with the test files excluded. The errors fall into four patterns:
  1. Stale fixtures missing fields after API type expansion (`ProviderResponse.{description, apiKey, baseUrl}`, `SkillResponse.instructions`, `AgentResponse.description`).
  2. Narrowed string-literal unions that tests use loosely (`"global"` not assignable to `MemoryScope`, `"code"` not assignable to `MemoryCategory`, `"root"` not in `SubscriptionScope`).
  3. `Array.prototype.at` usage — added in ES2022; `tsconfig.json` targets ES2020.
  4. General drift between test mocks and current type definitions.
- **Immediate fix (this PR):** Created `apps/ui/tsconfig.build.json` that extends `tsconfig.json` and excludes `**/*.test.ts(x)` + `**/*.spec.ts(x)` + `tests/**`. Updated `package.json` build script from `"tsc && vite build"` to `"tsc -p tsconfig.build.json && vite build"`. Production type-check + bundle now works; IDE/vitest still type-check tests via the base config.
- **Why this is "tech debt" not a real bug:** Production code passes type-check cleanly. Tests run via vitest (which has its own type behavior) regardless of this build-script exclusion. The errors are real but they reflect outdated test fixtures, not production correctness.
- **Followup fix (separate PR or piecemeal):** Update each test file's fixtures to match current type definitions. Bump `lib` to ES2022 to unblock `Array.prototype.at`. Tighten string-literal usage where types narrowed.
- **Status:** mitigated (build unblocked); test fixtures still need updates — fix opportunistically as files are touched, or batch as a cleanup PR.

---

## Phased fix plan

Each phase produces value standalone — none of them require finishing the next.

### Phase 0 — Real bugs (do anytime, independent)
**Closes:** TD-001
- One PR, one transaction wrapper, one regression test covering partial-failure (drop the connection between deletes and assert no half-archived state).

### Phase 1 — Knowledge-side trait shape (the unblock)
**Closes:** TD-010, TD-011
**Settles:** TD-013 design decision
- Define `KnowledgeGraphStore` trait. Decide its home: new `zero-knowledge-store` interface crate vs. trait in existing `services/knowledge-graph/`. Recommend the former because Phase 5 wants a clean factory boundary.
- Move existing `GraphStorage` into a `zero-knowledge-store-sqlite` impl (or feature-gate inside `services/knowledge-graph/`).
- Fix `CausalEdgeStore::new` to take a store handle, not a `Connection`.
- Resolve TD-013: pick (a) keep `VectorIndex` separate or (b) fold into the store traits. Recommend (b); document the choice in `docs/adr/decisions.md`.
- **This is the longest pole.** Without it, TD-012 and TD-014 are blocked.

### Phase 2 — Knowledge-side dialect cleanup ✅ done
**Closed:** TD-030, TD-031 (knowledge-side callsites only)
- Replaced `datetime('now')` with Rust-side `chrono::Utc::now()` parameter binding across `memory_repository.rs` (7), `procedure_repository.rs` (3), and `episode_repository.rs` (1) — 11 callsites total.
- TD-031 was already addressed by Phase 1's trait surface; Phase 2 audit confirmed all remaining `INSERT OR …` callsites are impl-internal.
- Follow-up TD-042 (deferred `julianday('now')` callsite) tracked separately for Phase 4.

### Phase 3 — Route shadow SQL through traits
**Closes:** TD-012, TD-014
- All 6 `gateway-execution/sleep/*` files route through `KnowledgeGraphStore` and `MemoryFactStore`.
- `services/execution-state/repository.rs` knowledge-side ops migrate to the new traits.
- Most code touched, but each file is independent — can be done as 6 small PRs.

### Phase 4 — Schema bootstrap per impl (no migration system)
**Closes:** TD-032
- Move existing SQLite inline migrations into `zbot-stores-sqlite/src/bootstrap.rs` (idempotent `CREATE TABLE IF NOT EXISTS …`).
- SurrealDB impl ships `zbot-stores-surreal/src/bootstrap.rs` (idempotent `DEFINE TABLE … IF NOT EXISTS`). _(Historical: implemented 2026-04-28, reverted in `d823b7bd`. The SQLite-side bootstrap is what remains.)_
- **No `Migrator` trait in `zbot-stores`.** Cross-version migration is explicitly out of scope — when actually needed, build a separate `zbot-stores-migrate` crate.

### Phase 5 — Switch wiring
**Closes:** TD-023
- `AppState` fields become `Arc<dyn …>`.
- `PersistenceFactory::new(config) -> AppStores` reads config and constructs the chosen knowledge-store impl.
- After this lands, adding SurrealDB is a new crate plus a config switch. _(Historical: the SurrealDB sidecar crate was added and then reverted in `d823b7bd`; the factory pattern stayed for cleanliness.)_

### Phase 6 (optional, deferred) — Conversations-side hygiene
**Closed:** TD-021, TD-022
**Still deferred:** TD-020
- ✅ Extracted `ConversationStore` and `OutboxStore` traits as hygiene scaffold (narrow surface; consumer migration to `Arc<dyn ...>` deferred to a future workstream alongside TD-023's retirement halves).
- ⏸ `DbProvider` / `StateDbProvider` reshape (TD-020) deferred — wide blast radius across service crates and not on the SurrealDB critical path.
- Strictly hygiene, not on the SurrealDB critical path. Can land after the swap.

---

## What we are deliberately NOT doing

To keep the registry honest and prevent scope creep:

- **Not touching the two-DB split.** `conversations.db` + `knowledge.db` is intentional and good. SurrealDB simply replaces `knowledge.db`. Don't merge them.
- **Not migrating any data yet.** This registry is about getting *new code* SurrealDB-ready. A one-time `knowledge.db → SurrealDB` migration tool is a separate workstream.
- **Not abstracting conversations storage** beyond Phase 6's hygiene cleanup. SQLite is the future-state for that subsystem.
- **Not introducing `sqlx`, `sea-orm`, or another ORM.** SurrealDB's Rust client speaks records-not-SQL; rusqlite stays for the SQLite impl.
- **Not building a "compatibility layer" between SQLite knowledge.db and SurrealDB.** Per-backend impls only. Dual-write / fall-back is explicitly out of scope.

---

## Reading guide

- **Architecture overview:** `docs/architecture/architecture.md`
- **Design decisions log:** `docs/adr/decisions.md` (record any non-obvious decisions about persistence boundaries here — particularly the TD-013 outcome)
- **SurrealDB future state:** to be added under `docs/architecture/future-state/` once Phase 1 design is settled
- **General code-health scan:** `docs/architecture/sonar_scan_report.md`

---

## How to add an item

1. Pick the next free `TD-NNN` ID within the relevant block (real bugs in 001–009, knowledge-side traits in 010–019, conversations-side traits in 020–029, dialect in 030–039, code smells in 040+).
2. Fill out: location (file:line), what it is, why it's debt, the fix approach, severity, scope tag, status.
3. If it slots into an existing phase, list it there. If not, add a new phase and explain why it doesn't fit existing ones.
4. Commit the doc change as part of the PR that introduces the related work, not as a doc-only commit. The registry should track reality.
