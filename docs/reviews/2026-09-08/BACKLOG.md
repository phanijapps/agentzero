# Improvement Backlog — 2026-09-08 Review

Four parallel reviews (DRY, tech debt, architecture, agent capability). Full
reports in this directory. This is the synthesis and recommended order.

## The big picture

The execution layer is healthy after op_clean_crap (typed errors, golden
traces, ExecCtx, hook framework). The debt has **migrated down into the
persistence layer** and **sideways into moved-but-never-split monoliths**.
Meanwhile the agent's learning loops are half-built: memory has recency data
it never uses, failure episodes are stored but never recalled, and the agent
can't write skills.

## Consolidated backlog (ranked)

### Tier 1 — quick wins, <1 day each, ~900–1,100 lines deletable

| # | Item | Source | Effort | Payoff |
|---|---|---|---|---|
| 1 | **Delete dead `distillation → gateway-events` dep** | arch | 5 min | removes an inversion for free |
| 2 | **`ErrorResponse` ×9 → 1 + `require()` helper for 503 guards** | dry #4/#5 | 1–2 h | ~260 lines, wire-format drift risk gone |
| 3 | **Cosine similarity ×11 → `agent-primitives::vec_math`** | dry #1 | 2 h | ~150 lines, kills f32/f64 recall divergence |
| 4 | **35+ stale `#[allow(dead_code)]` purge** (several provably false) | debt H4 | 2 h | stops masking real dead code |
| 5 | **Failed-episode avoid-list** — surface failed episodes' `key_learnings` at session start | agent #1 | S | agent stops repeating known failures |
| 6 | **Structured failure feedback** — feed repeated tool errors back as context instead of one nudge | agent #2 | S | fewer wasted turns |
| 7 | **`recall_facts_prioritized` → one-line delegation** | dry #7 | 15 min | removes trait-default circularity |
| 8 | **Distillation supersede/fact-literal helpers** | dry #3 | 2–3 h | ~150 lines, one place for supersede policy |
| 9 | **`make_state()` test scaffolding ×3 → shared module** | dry #10 | 30 min | ~40 lines |

### Tier 2 — focused projects, ~1 week each

| # | Item | Source | Payoff |
|---|---|---|---|
| 10 | **`StoreError` type for store traits** — `Result<_, String>` starts at trait level, 400+ sites | debt H2 | unblocks everything below; typed persistence boundary |
| 11 | **Recall monolith split** — `recall/mod.rs` 4,249 lines with a 676-line function on the hottest read path | debt H1 | maintainable recall; add recency/usage scoring here (agent #3) |
| 12 | **`ProviderServiceLlmFactory` promotion + 6-site migration** | dry #2 | ~200 lines; factory exists, call sites ignore it |
| 13 | **`GraphStorage::run/tx` wrappers** — 42× double-closure ritual in 5,467-line file | dry #8 | ~200 lines, biggest readability win in largest file |
| 14 | **`WriteSkillTool`** — agent writes its own skills after successful sessions | agent #4 | strongest learning loop; closes session→skill cycle |
| 15 | **`ward_artifact_indexer` → services/** — 987 lines, zero execution deps | debt H6/arch | gateway-execution slims further |
| 16 | **`VaultPaths` → `agent-primitives`** — kills 6 of 12 upward store→gateway edges in one edit | arch #1 | dependency graph un-inverts |
| 17 | **Recency + usage scoring in recall** — `updated_at` stored but never used in ranking | agent #3 | fresh corrections outrank stale facts |

### Tier 3 — bigger bets, schedule deliberately

| # | Item | Source | Payoff |
|---|---|---|---|
| 18 | **Typed store traits** (kill the JSON-`Value` hop) — ~32 encode/decode sites; one PR per trait, conformance harness proves parity | dry #6 | compile-time safety at persistence boundary |
| 19 | **`zbot-stores` facade retirement** — 979-line re-export shim, 63 files still on facade vs 88 on real crate | arch #2 | finish the half-done split |
| 20 | **`zbot-stores-sqlite` de-composition-rooting** — 21,686 lines wiring api-logs/execution-state/gateway-services; composition belongs in gateway shell | arch #3 | crate becomes what its name says |
| 21 | **Gateway shell actually a shell** — 31,859 lines incl. 44 HTTP files, 50-field AppState (2,052 lines), a2a_tasks + durable_agent_tasks subsystems | arch #4 | AppState decompose; subsystems → services |
| 22 | **Parallel subagent children** — `parallel: false` hardcoded at spawn.rs:2079/2282 despite `DelegationContext.parallel` existing | agent #6 | fan-out delegation |
| 23 | **Web search tool** (SearxNG or similar, zero-config) | agent #5 | agent gets internet |
| 24 | **Distillation crate split** — 3,091-line monolith moved but not decomposed | debt H5 | finishes the extraction properly |
| 25 | **Shared LLM-JSON parser** in agent-runtime — 3 independent fence-stripping stacks | dry #9 | next structured-output feature doesn't grow a 4th copy |

### Explicitly not doing now

- Critic pass before respond (agent #8) — L effort, unclear payoff until failure feedback (#6) proves insufficient
- Hierarchical summaries (agent #9) — L, context_policy budget-reject works today
- Rebuild of memory engine — hybrid RRF + MMR + query gate already strong; improve scoring incrementally instead

## Already strong (don't touch)

- Hybrid recall: RRF fusion + MMR diversity + query gate
- Procedures with concrete args
- Strategy emergence from failure clustering
- Steering queue, checkpoint versioning, offload-large-results
- Execution layer: typed errors, golden traces, hook framework

## Reports

- [review-dry.md](review-dry.md) — 10 findings, file:line evidence
- [review-debt.md](review-debt.md) — 9 hotspots ranked by severity
- [review-arch.md](review-arch.md) — dependency graph audit, 8-step migration order
- [review-agent.md](review-agent.md) — 9 agent-capability improvements ranked by impact/effort
