# Tech Debt Review — AgentZero (op_clean_crap @ c9ed8e81)

Read-only audit. Method: rg/find/wc sweeps + spot-checks of the largest files.

## Summary

The execution layer (gateway-execution, agent-runtime) is in good shape after the recent consolidation — typed `ExecutionError`, ExecCtx, hooks, golden traces. The remaining debt clusters in four places:

1. **Persist-layer god files** (stores-sqlite, engram-adapter, execution-state) — untouched by the cleanup waves.
2. **The recall monolith** — `gateway-memory/src/recall/mod.rs` is 4,249 lines with a 676-line single function.
3. **Stale `#[allow(dead_code)]`** — 35+ sites; several are now false (the code IS used cross-crate) and hide genuinely dead scaffolding.
4. **Stringly-typed store traits** — `Result<_, String>` starts at the trait level (`zbot-stores-traits`) and propagates to 20+ files (400+ sites). The ExecutionError fix never reached the stores layer.

Also: `services/distillation` was moved but not split — `lib.rs` is still a 3,091-line monolith including 1,400 lines of tests and the dedup/governance logic that the extraction plan intended to relocate.

---

## Hotspots

### H1. `recall/mod.rs` — 4,249-line file, 676-line function
- **Severity:** Critical
- **Files:** `gateway/gateway-memory/src/recall/mod.rs`
- **Evidence:** `recall_unified_outcome_with_visibility` spans lines 693–1369 (676 lines). `recall()` itself is 175 lines. Sibling modules exist (query_gate.rs, scored_item.rs, mmr.rs…) but the core path stayed in mod.rs. Only 2 of 4,249 lines deviate from the "small focused modules" pattern the crate already established.
- **Why it hurts:** Any recall change requires understanding a 676-line visibility/fusion pipeline. Untestable in isolation; the crate's own tests (100+ lines each) exist only because the function can't be decomposed.
- **Fix:** Extract phases as pure functions: `resolve_visibility`, `fuse_sources`, `apply_gates`, `rank`. The 175-line `recall()` orchestrates. Mirror the distillation decomposition pattern (phase functions, each <80 lines).
- **Effort:** M

### H2. Stringly-typed store traits — 400+ `Result<_, String>` sites
- **Severity:** High
- **Files:** `stores/zbot-stores-traits/src/memory_facts.rs` (44 sites, trait-level), `zbot-engram-adapter/src/stores/sidecars.rs` (73), `memory_facts.rs` (69), `execution-state/src/service.rs` (67), `repository.rs` (51), `agent-tools/src/tools/memory.rs` (49), +14 more files
- **Evidence:** The trait contract itself returns `Result<Value, String>` (memory_facts.rs:134,144,166). Adapters and services then string-match on error text.
- **Why it hurts:** The ExecutionError work (155 sites fixed in gateway-execution) stopped at the crate boundary. Store errors can't be matched, unwrap into generic 500s, and force `.map_err(|e| e.to_string())` noise at every call site. This is the single largest remaining class of debt.
- **Fix:** Define `StoreError` enum in `zbot-stores-traits` (or a shared `zbot-stores-domain` error). Change trait signatures first, then fix impls mechanically (sqlite + engram adapters), then delete the `map_err` shims at call sites.
- **Effort:** L (mechanical but wide — trait ripple touches ~20 files)

### H3. Persist-layer god files
- **Severity:** High
- **Files:**
  - `stores/zbot-stores-sqlite/src/kg/storage.rs` — 5,467 lines, 145 unwraps (non-test), 8+ functions >100 lines (`get_neighbors` 230, `store_entity` 143, `find_duplicate_candidates` 139, `compute_lca_path` 110)
  - `services/execution-state/src/repository.rs` — 4,156 lines, **230 unwraps**, 132 expects, long fns (`row_to_session_message` 170, `get_dashboard_stats` 127, `complete_session_plan` 122)
  - `runtime/agent-tools/src/tools/memory.rs` — 3,304 lines, 79 unwraps (`action_recall` 178 lines)
  - `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs` — 2,750 lines
  - `runtime/agent-tools/src/tools/ward.rs` — 2,738 lines, 116 unwraps
- **Evidence:** wc -l sweep; unwrap counts exclude tests. storage.rs functions mix SQL string building, row mapping, and graph semantics in single bodies.
- **Why it hurts:** These are the persistence backbone — every runtime path funnels through them. 230 unwraps in repository.rs means any malformed row panics a tokio worker mid-request. Untestable SQL blobs.
- **Fix:** Per file: split row-mapping from SQL from semantics; replace unwraps with typed `StoreError` (ties into H2); storage.rs becomes `kg/{queries,rows,graph_ops}.rs`. Do execution-state first (highest unwrap density, hottest path).
- **Effort:** L (per-file M)

### H4. Stale/misleading `#[allow(dead_code)]` — 35+ sites
- **Severity:** Medium
- **Files:** `runtime/agent-tools/src/tools/{goal,ingest,graph_query}.rs` (file-level `#![allow(dead_code)]` — but GoalTool/IngestTool/GraphQueryTool ARE registered in `gateway-execution/src/invoke/builder.rs:1080-1092`), `runtime/agent-runtime/src/progress.rs` (4 sites, "kept for diagnostics/legacy"), `gateway/src/http/conversations.rs` (dead route handler + 4 TODOs "Phase 3b"), `stores/zbot-engram-adapter/src/{bootstrap,semantic_services}.rs` ("prepared", "no runtime consumer approved"), `gateway/src/state/mod.rs:1684`, `gateway/src/websocket/subscriptions.rs` (2)
- **Evidence:** grep sweep. The agent-tools pragmas are actively harmful: they suppressed the warning that would have flagged genuinely dead helpers *inside* files whose entry types are alive.
- **Why it hurts:** The project's stated rule is "no allow(dead_code), delete dead code immediately" — these are escape hatches that accumulated. "Prepared for future" adapters (engram semantic_services) are speculative scaffolding.
- **Fix:** Delete file-level pragmas in goal/ingest/graph_query (types are pub + consumed; the allow does nothing but mask internal rot). For each site: either the code is used (remove pragma) or it isn't (delete code — `ProgressTracker` legacy fields, conversations.rs stub routes, engram "prepared" surfaces, subscriptions dead fields).
- **Effort:** S per site, M total

### H5. `services/distillation/src/lib.rs` — moved but not split
- **Severity:** Medium
- **Files:** `services/distillation/src/lib.rs` — 3,091 lines, 101 fns, tests start at 2325 (~766 lines of tests inline)
- **Evidence:** The extraction plan (distillation-store-extraction, deleted with the spec cleanup) called for `distiller.rs`, `extract.rs`, `graph.rs`, `strategy.rs`, `wiki.rs`. Only `wiki.rs` exists. `upsert_facts_with_dedup` (141 lines) and `project_distilled_graph` (129 lines) still carry the supersede/governance logic in-crate.
- **Why it hurts:** The move achieved crate isolation but not cohesion — lib.rs is a vertical slice of everything. New contributors can't find the LLM contract vs. the store policy without reading 1,500 lines.
- **Fix:** Split along existing seams: `types.rs` (Extracted* structs, ~150 lines), `distiller.rs` (SessionDistiller + distill orchestration, ~300), `graph.rs` (project + governance helpers), `facts.rs` (dedup/supersede), `strategy.rs`, move tests to `tests/`.
- **Effort:** M

### H6. `ward_artifact_indexer.rs` — misplaced module
- **Severity:** Medium
- **Files:** `gateway/gateway-execution/src/ward_artifact_indexer.rs` — 987 lines
- **Evidence:** Header comment: "scans a ward for structured files (JSON) after a session completes… emits entities… Zero LLM cost." It depends on `KgEpisodeStore` + `KnowledgeGraphStore` traits only — no execution machinery (no ExecCtx, no events, no runner). Sibling `indexer/relationship_rules.rs` carries a stale `#![allow(dead_code)]`.
- **Why it hurts:** It's a post-session ingestion service squatting in the execution crate. It inflates gateway-execution's surface and couples unrelated change cadence. Same story as distillation before its extraction.
- **Fix:** Move to `services/` (new `services/ward-indexing/` or fold into `services/knowledge-graph/` since it only produces KG entities/episodes). Also resolves its ExecutionError dependency (give it a local error type like distillation did).
- **Effort:** S–M

### H7. Distillation pulls gateway crates into services/
- **Severity:** Medium
- **Files:** `services/distillation/Cargo.toml` → `gateway-events`, `gateway-services`, `gateway-templates`
- **Evidence:** Only services crate depending on gateway/* (checked all six). Root AGENTS.md dependency order says services sit below gateway.
- **Why it hurts:** Inverts the layering the workspace documents. gateway-services is a grab-bag (ProviderService, VaultPaths, AgentService…) so the pull is wide: distillation transitively drags most of the gateway layer.
- **Fix:** Define narrow ports in distillation (`trait DistillationEvents`, `trait ProviderLookup`, `trait VaultLayout`), implement them in gateway, wire at startup. Alternatively relocate distillation under gateway/ if services/ is meant to stay gateway-free.
- **Effort:** M

### H8. TODO catalog — 14 markers, two clusters
- **Severity:** Low
- **Evidence:**
  - `gateway/src/http/conversations.rs` — 4× "Connect to daily_sessions in Phase 3b" (stub routes + H4 dead_code)
  - `services/daily-sessions/src/summary.rs` — 2× "Integrate with LLM" (placeholder summaries shipped as real)
  - `runtime/agent-runtime/src/mcp/manager.rs:55` — "Implement from existing code"
  - `ward_artifact_indexer.rs` — 2× "Phase 6b" EntityType::Event upgrades
  - remainder: doc-adjacent comments
- **Fix:** conversations.rs stubs: implement or delete routes. daily-sessions summary is a user-visible placeholder — wire to an LLM call or drop the endpoint.
- **Effort:** S each

### H9. Non-test `expect()` density in gateway-execution
- **Severity:** Low
- **Files:** `peer_messaging.rs` (65), `delegation/spawn.rs` (59), `gateway-services/src/skills.rs` (43), `invoke/batch_writer.rs` (28), `artifacts.rs` (27)
- **Evidence:** Many are lock-acquisition (`lock().expect("poisoned")`) and channel sends — defensible. But peer_messaging/spawn mix them with store calls whose failure should degrade, not panic.
- **Fix:** Audit per-site: locks can stay (poisoning = real bug), store/IO expects become `?` with ExecutionError.
- **Effort:** S

---

## Top 5 — fix first

1. **H2 — `StoreError` for the store traits.** Unblocks H3's unwrap purge (errors become typed), kills 400+ stringly sites, completes the typed-error work already proven in gateway-execution. Highest leverage per line changed.
2. **H1 — decompose `recall_unified_outcome_with_visibility`.** 676 lines on the hottest read path (every message recalls memory). The crate already has the module pattern; this is the last monolith in it.
3. **H4 — purge stale `allow(dead_code)`.** Cheapest win, restores the compiler as the dead-code oracle, deletes a few hundred lines of scaffolding. Do it before H5 so the distillation split starts clean.
4. **H3a — `execution-state/repository.rs` unwrap purge + split.** 230 unwraps on the state backbone; panics here kill requests mid-flight. Combine with H2 rollout for this crate first.
5. **H5 + H6 — finish the extraction pattern.** Split distillation lib.rs into its planned modules and relocate ward_artifact_indexer to services/. Both are completing work already committed to, not new architecture.

## Non-findings (checked, healthy)

- gateway-execution Cargo deps match the documented dependency order (no layering violation found post-distillation-move — the violation is on distillation's side, H7)
- TODO/FIXME volume is genuinely low (14) for a 239k-line workspace
- Golden traces + 544 gateway-execution tests + typed ExecutionError make the execution layer regression-safe for the H-series refactors above
