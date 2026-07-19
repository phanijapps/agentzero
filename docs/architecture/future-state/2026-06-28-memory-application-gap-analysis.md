# Memory-Application Gap Analysis

**Date:** 2026-06-28 · **Author:** analysis pass (code + DB evidence) · **Branch:** `rig-engine-migration-spec`
**Scope:** why zbot *collects* knowledge well but *applies* it "hit and miss" — root causes + additive-only fixes.
**Compared against:** `~/Documents/zbot/wards/agent-memory-framework/memory-vs-knowledge-architecture/memory-knowledge-architecture.md` (the composable modular target).

> **Guiding constraint (user):** nothing is removed. Every gap below is closed by *adding* a wire, a writer, a renderer, or a store column. The most valuable fixes reuse infrastructure that already exists but isn't connected.

---

## 0. The diagnosis in one paragraph

zbot's **write side is healthy and growing** — distillation runs on 88% of sessions and has produced 710 facts / 714 entities / 654 relationships / 105 episodes; `memory_facts` is genuinely bi-temporal (1,146 rows, 45 superseded, growing ~165–320/day); beliefs are synthesized (307) and contradictions detected (13). **The read side — the Retrieval Layer that turns stored knowledge into applied knowledge — is where it breaks.** Three independent failure clusters explain the user's three complaints:

1. **"Hit and miss"** → recall silently drops borderline-relevant facts (`min_score=0.3` vs SQLite-vec scores that routinely land 0.05–0.25), shows no confidence/provenance so the LLM can't weight results, bypasses its own Query Router, runs with a dead goal-alignment booster, and reaches only **~18% of sessions**. It is also **unobservable** — `recall_log` has 0 rows because its writer is a no-op trait default that is never called.
2. **"Plans get read all upfront"** → the root orchestrator's `plan.md` is injected **verbatim** on every continuation *and* re-rendered every turn by `PlanBlockMiddleware` (flagged `is_summary=true` so summarization can't touch it). There is **no capacity budget**; root sessions balloon to **p90 = 1.09M, max = 11M tokens**.
3. **"Same tool failures resurface"** → `failure_count = 0` for **all 202 procedures**. Pattern mining reads successful episodes only; tool failures never become procedures; the only failure-learning is substring-matched `correction` facts surfaced *after* the failure already happened; distillation's prompt explicitly tells the LLM to skip transient errors.

A fourth, cross-cutting cluster: several closed-loop stores are **write-only** — beliefs are never superseded (0/307), the 13 contradictions are **all open / 0 resolved**, the KG side is append-only (no `valid_until`), inter-cluster relationships were never synthesized, `kg_episode_payloads` is empty (0 bytes despite 70 "done" episodes), and there is **no read-side access tracking anywhere** (`access_count = 0` on all entities/relationships).

**Headline:** zbot accumulates knowledge enthusiastically but has weak evidence it *uses* or *reconciles* it beyond fact supersession. The fixes below are overwhelmingly "connect what's already built."

---

## 1. What is working (keep, build on this)

| Capability | Evidence |
|---|---|
| Distillation pipeline | 117 runs, 88% session coverage, 710 facts / 714 entities / 654 rels / 105 episodes. ~16.6 s/run. The 8 "failed" runs are `"Distillation in progress"` races, not real failures. |
| `memory_facts` bi-temporality + supersession | 1,146 facts; 1,126 have `valid_from`; 45 superseded; avg confidence 0.92. Genuinely bi-temporal. |
| Cross-store ranking | RRF fusion + MMR diversity + category weights + ward boost + temporal decay + contradiction/supersession penalties across 9 subsystems (`scored_item.rs`, `mmr.rs`). |
| Ward-entry correction injection | 15 ward switches across 12 sessions auto-inject the "Rules from past corrections" block — **the strongest "memory being applied" signal in the data.** |
| Micro-recall | Reactive retrieval on delegation / tool-error / ward-entry / entity-mention triggers (`micro_recall.rs`). |
| Belief synthesis | 307 beliefs with embeddings + `source_fact_ids` provenance; contradiction detector fires. |
| Hierarchy builder (entity half) | 98 entities at `layer>0` with `parent_cluster_id` — H-3 ran. |
| On-demand recall tool | `memory(action="recall")` returns scored, formatted top-k (23 real calls last week). |

---

## 2. Gaps by complaint

### 2.A — "Knowledge application is hit and miss" (Retrieval Layer)

| # | Gap | Evidence (file:line / DB) |
|---|---|---|
| A1 | **`min_score = 0.3` silently drops borderline facts.** SQLite-vec hybrid scores for legitimately relevant facts routinely land 0.05–0.25; the test suite must use `relaxed_recall_config()` with `min_score:0.0` to get results through. This is the single biggest "miss." | `gateway-memory/src/lib.rs:270` (default; overridable via `recall_config.json`); `recall/mod.rs:385`, `recall/mod.rs:334` |
| A2 | **No confidence/provenance rendered** for non-belief items. `format_scored_items` emits `- [fact] {content}` — no score, source, or recency. Beliefs already show `[belief <conf>]`; the data is in `ScoredItem.provenance` for everything else but is discarded. | `gateway-execution/src/recall/mod.rs:36-69`; verified |
| A3 | **Query Router bypassed on the main path.** `QueryGate` (`Skip`/`Direct`/`Split`) exists and is wired into legacy `recall()` — but the bootstrap default `recall_unified` queries all 9 subsystems unconditionally, even for "thanks!". | `recall/query_gate.rs`; `recall/mod.rs:347-355` (no gate) vs `recall/mod.rs:196-199` (legacy uses it) |
| A4 | **Goal-alignment booster is dead on the main path.** `active_goals` is passed as `&[]` at both bootstrap recall calls, so `intent_boost` (designed to prioritize goal-relevant items 1.3×) never fires — even though goals are already listed one call later. | `invoke_bootstrap.rs:473, 528` (`&[]`); `scored_item.rs:105-122` (intent_boost); `invoke_bootstrap.rs:578-589` (goals already loaded) |
| A5 | **Embedding-only subsystems silently empty on embed failure.** Wiki/procedures/beliefs/graph all `match Some(emb) => …, _ => Vec::new()`. If the embedder rate-limits or the input trips a context-length error, those stores vanish with no "recall degraded" signal. | `recall/mod.rs:393, 406, 432, 610` |
| A6 | **Recall reaches only ~18% of sessions** (16 of 87 tool-using sessions). It is opportunistic (via the `memory(recall)` tool), not systematic. | conversations.db: 23 recall calls / 16 sessions |
| A7 | **Recall is unobservable.** `recall_log` has **0 rows** — its writer (`RecallLogRepository::log_recall`) exists, but the trait method is a **default no-op** (`async fn log_recall(&self, _session_id, _fact_key)`) that is never invoked on the recall path. You cannot answer "which facts does the system actually rely on?" from the DB. | `zbot-stores-traits/src/auxiliary.rs:58`; `zbot-stores-sqlite/src/recall_log_repository.rs:28` |

### 2.B — "Plans get read all upfront by the orchestrator" (Context Composer)

| # | Gap | Evidence |
|---|---|---|
| B1 | **Plan injected verbatim on every continuation.** `build_continuation_message` `read_to_string`s the entire `plan.md` and embeds it whole — no summarization, no truncation, no eviction. | `runner/core.rs:272-305`, `find_latest_plan` at `:1602-1648` |
| B2 | **Plan re-rendered every turn** by `PlanBlockMiddleware`, flagged `is_summary = true` so summarization middleware can't touch it. Full plan text persists for the entire session. | `runtime/agent-runtime/src/middleware/plan_block.rs:49-64`; `executor.rs:414-419` |
| B3 | **No capacity budget for recall/context.** Recall is concatenated into history with no token accounting; the only backstop is reactive `ContextEditingMiddleware` firing at **70–80%** of the window — by which point tool results/ recall have already been truncated. Root sessions hit **p90 = 1.09M, max = 11M tokens**. | `invoke_bootstrap.rs:470-600`; `executor.rs:387-412` (reactive only) |
| B4 | **On-demand plan infra exists but is pre-empted.** Subagents get a self-closing `<session_ctx …/>` pointer + `ctx.<sid>.plan` fact; the orchestrator is *not* given that contract — it gets the plan inlined instead. The fix is routing, not new storage. | `session_ctx/preamble.rs:39-84`; `session_ctx::writer::plan_snapshot` at `runner/core.rs:294` |

### 2.C — "Same tool failures resurface" (Procedural Memory loop)

| # | Gap | Evidence |
|---|---|---|
| C1 | **No writer turns a tool failure into a procedure / structured rule.** `failure_count = 0` for all 202 procedures. Pattern mining reads successful episodes only. The whole write path is gated on `outcome == success`. | `sleep/pattern_extractor.rs:138` (success-only); DB: `SELECT COUNT(*) FROM procedures WHERE failure_count>0` → 0 |
| C2 | **No procedure retrieval at tool-selection time.** The only consumer of `recall_procedures` is `intent_analysis`, matched against the **user message** at session start — not against tool state. Tool selection inside a turn is stateless w.r.t. procedures. | `recall/mod.rs:159-178`; `intent_analysis.rs:553-658` |
| C3 | **The only failure-learning is substring-matched, post-failure.** `handle_tool_error` pulls `get_facts_by_category("correction",10)` and filters by `content.contains(tool_name)` — no vec similarity, no procedures, runs *after* the failure. | `invoke/micro_recall.rs:222-254` |
| C4 | **Distillation suppresses error signal by prompt design.** The prompt says "Skip ephemeral details (…transient errors…)." There is no dedicated "tool-failure" extraction. Result: only 48 corrections, most written once. | `templates/distillation_prompt.md:216` |
| C5 | **Real recurring failures the agent doesn't learn from:** `multimodal_analyze` 400 (4/11 invocations), `delegate_to_agent` DELEGATION_TOO_LARGE (>4000 chars), `memory` FACT_TOO_LONG (>500 chars), shell-write guard. | conversations.db execution_logs |

### 2.D — Closed-loop / write-only stores (Belief Network + bi-temporal + hierarchy)

| # | Gap | Evidence |
|---|---|---|
| D1 | **Belief contradictions never resolved.** 13 contradictions, **13 OPEN, 0 resolved**. Detector runs (B-2 ✓); resolver (B-3) does not. 0 beliefs superseded, 0 stale. | knowledge.db `kg_belief_contradictions` |
| D2 | **KG side is append-only — bi-temporality is half-real.** `memory_facts` is bi-temporal; `kg_beliefs` never sets `valid_until`/supersedes; `kg_relationships` sets `valid_from` but never `valid_until`/invalidates; `kg_entities` **doesn't set `valid_from` at all.** | knowledge.db |
| D3 | **Inter-cluster relationships never synthesized.** 98 entities at `layer>0` (H-3 ran) but **0 relationships are layer>0 / `is_inter_cluster=1`** (H-4 didn't run on relationships). Only the entity half of hierarchical memory exists. | knowledge.db |
| D4 | **`kg_episode_payloads` is empty** — 70 "done" episodes, 0 bytes of payload. `kg_episodes.started_at` always NULL (status skips running→done). | knowledge.db |
| D5 | **No read-side access tracking anywhere.** `access_count = 0` for all 457 entities / 746 relationships; `last_accessed_at` NULL everywhere; `memory_facts`/`kg_beliefs`/`procedures` have no access columns at all. Nothing records that a fact was *read*. | knowledge.db |
| D6 | **Dead / never-started stores:** `kg_causal_edges` (0), `kg_goals` (0), `memory_facts_archive` (0). `skill_index_state` indexed once on 2026-06-22, never refreshed (stale 7 days vs. an otherwise-live DB). | knowledge.db |
| D7 | **Data-quality nits:** entity types `Concept` vs `concept` stored as separate types (98 vs 71); `correction` appears both as a `category` (48) and an `epistemic_class` (1) — likely a writer conflating two axes. | knowledge.db |

---

## 3. Comparison vs the composable modular architecture

Target = 5 layers (Orchestration / Retrieval / Memory / Knowledge / Storage). Status of each component in zbot today:

| Target component | zbot status | Notes |
|---|---|---|
| Orchestration | ✅ Present | agent loop, executor, middleware chain |
| Retrieval — Query Router | 🟡 Partial | `QueryGate` exists but bypassed on `recall_unified` (A3) |
| Retrieval — Ranker | ✅ Present | RRF + MMR + weights + decay + supersession |
| Retrieval — Provenance Tracker | 🟡 Partial | captured on every item, **not rendered** for non-beliefs (A2) |
| Retrieval — Context Composer | ❌ Missing | no capacity budget; reactive editing only at 70–80% (B3) |
| Retrieval — Predictive | 🟡 Partial | event-driven micro-recall; no prediction-error trigger (A5) |
| Memory — Working | ✅ Present | working-memory middleware |
| Memory — Episodic | ✅ Present | episodes, session_episodes, distillation |
| Memory — Semantic | ✅ Present | `memory_facts`, bi-temporal |
| Memory — Procedural | 🟡 Partial | table + vec index; **no failure loop, no tool-time match** (C1–C4) |
| Knowledge — Flat Facts | ✅ Present | `memory_facts` |
| Knowledge — Taxonomy/Hierarchy | 🟡 Partial | entity hierarchy only; **no inter-cluster relationships** (D3) |
| Knowledge — Vector | ✅ Present | sqlite-vec across stores |
| Knowledge — Graph | 🟡 Partial | populated but **append-only** (D2) |
| Consolidation pipeline | 🟡 Partial | compactor/pruner/pattern run; **belief resolution + episode payloads missing** (D1, D4) |
| Storage tiers (T1–T5) | ✅ Present | context / sqlite / vec / graph / archive |

**Pattern:** the *storage* and *write* halves of every layer are present; the *read-side integration* (Router, Provenance rendering, Composer, Procedural match-action, Consolidation resolution) is the missing half. That is exactly the "composable but not yet composed" gap.

---

## 4. Additive improvement proposals

Prioritized by **confidence × impact ÷ effort**. Tier 1 = connect existing infrastructure (lowest risk). All are additions; none remove existing behavior.

### Tier 1 — Wire up what already exists (highest confidence, do first)

| # | Proposal | Closes | Files | Effort |
|---|---|---|---|---|
| **P1** | **Render provenance + confidence in `format_scored_items`** for non-belief items: `- [fact, conf 0.9, schema] {content}` using `item.provenance.source` + `item.score` (data already in `ScoredItem`). | A2 | `gateway-execution/src/recall/mod.rs:47-54` | ~10 lines |
| **P2** | **Invoke `log_recall` on the recall path.** Override the no-op trait default / call the existing `RecallLogRepository::log_recall` after each successful recall. Restores observability of what the system relies on. | A7 | `zbot-stores-traits/src/auxiliary.rs:58`; call site in `recall/mod.rs` | ~15 lines |
| **P3** | **Wire `QueryGate` into `recall_unified`** — consult `query_gate.reformulate(query)` at the top and fan out subsystems by decision (`Skip`→corrections+high-conf facts only). Reuses the exact pattern legacy `recall()` already uses. | A3 | `recall/mod.rs:347-355` | ~25 lines |
| **P4** | **Thread `active_goals` into bootstrap recall** — replace `&[]` with a `GoalLite` projection from the `goal_adapter.list_active()` call already made a few lines later. Activates the dead `intent_boost` (1.3× for goal-aligned items). | A4 | `invoke_bootstrap.rs:473, 528` | ~15 lines |
| **P5** | **Replace the hard `min_score` floor with a two-tier soft floor.** Instead of dropping items < 0.3 outright, keep a small "low-confidence — verify" tail (tagged as such via P1) so borderline-but-relevant facts survive. Default floor tunable, already overridable via `recall_config.json`. | A1 | `recall/mod.rs:385, 334`; `lib.rs:270` | ~30 lines |
| **P6** | **Emit a "recall degraded" marker when an embedding-only subsystem empties on embed failure** (e.g. `## Recalled Context (partial — embedder unavailable, beliefs/graph skipped)`). | A5 | `recall/mod.rs:393, 406, 432, 610` | ~20 lines |

### Tier 2 — Close the procedural-memory loop (addresses "tool failures resurface")

| # | Proposal | Closes | Files | Effort |
|---|---|---|---|---|
| **P7** | **Tool-failure → procedure/correction writer** on `handle_tool_result`. When a tool errors (or shell-style `success==false`), fire-and-forget a structured write keyed by `(agent, ward, tool, normalized_error_signature)` — increment `failure_count` via the existing `ProcedureStore::increment_failure`, or upsert a `tool.{tool}.{sig}` correction fact. Mirrors the existing `extract_and_persist` spawn. | C1, C5 | `gateway-execution/src/runner/execution_stream.rs:218-225` | ~60 lines |
| **P8** | **Pre-tool-call procedure recall.** New `MicroRecallTrigger::PreToolCall { tool_name, args_summary }` detected *before* dispatch; new `recall_procedures_by_tool_state(tool, error_sig, ward)` that vec-searches `procedures_index`. Surfaces "last time shell ran sudo here it was refused — use --user" *before* the retry. | C2 | `recall/mod.rs:159`; `micro_recall.rs`; `execution_stream.rs:255` | ~80 lines |
| **P9** | **Failure-aware pattern mining.** Add a `list_failed_episodes_with_embedding` pass to `PatternExtractor`; when failed episodes share a tool-call prefix ending in the same error signature, synthesize a **negative procedure** (`success_count=0, failure_count=N`). The existing `intent_analysis` success-rate tiering then surfaces it as a "known-bad approach" advisory. | C1 | `sleep/pattern_extractor.rs:138, 134-150` | ~70 lines |
| **P10** | **Distillation: extract tool-failure facts under a dedicated category.** Add a `tool_failure` extraction section to the prompt and *un-exclude* tool failures from the "skip ephemeral" rule. Distillation's writer already routes through `upsert_typed_fact`, so reinforcement + embedding come free. | C4 | `templates/distillation_prompt.md:216`; `distillation.rs:705-732` | prompt + ~20 lines |
| **P11** | **Reinforce procedures from live tool outcomes.** After a tool succeeds/fails, look up procedures whose first step matches the tool and call the existing `increment_success`/`increment_failure`. Columns already exist; they're just starved of writes (only `run_procedure` feeds them today). | C1 | `execution_stream.rs::handle_tool_result` | ~40 lines |

### Tier 3 — Context Composer + plan-on-demand (addresses "plans read upfront")

| # | Proposal | Closes | Files | Effort |
|---|---|---|---|---|
| **P12** | **Budgeted recall.** New `recall_unified_budgeted(…, token_budget)` that truncates the RRF-merged list by estimated tokens (4-char/token heuristic) rather than by item count. Pass the already-computed `context_window_tokens` into recall so it never pushes the prompt toward the 70% reactive-edit cliff. | B3 | `recall/mod.rs`; `invoke_bootstrap.rs:470-503` | ~60 lines |
| **P13** | **Orchestrator plan-on-demand.** Route the orchestrator to the *existing* `ctx.<sid>.plan` fact (subagent contract) instead of inlining `plan.md` verbatim; on continuation inject a compact pointer + a one-line diff vs. last-seen plan rather than the full text. Lets summarization/eviction apply to plan content too. | B1, B2, B4 | `runner/core.rs:272-305`; `session_ctx/preamble.rs`; `plan_block.rs` | ~100 lines |
| **P14** | **Predictive / earlier context editing.** Move from reactive editing at 70–80% to a budget-aware composer that summarizes oldest tool-result blocks *before* recall is truncated — keep recall, summarize raw logs. (Composes with P12.) | B3 | `executor.rs:387-412` | ~120 lines |

### Tier 4 — Close the closed-loop stores (Belief Network roadmap + bi-temporal + hierarchy)

| # | Proposal | Closes | Files | Effort |
|---|---|---|---|---|
| **P15** | **Wire the belief conflict resolver (B-3).** 13 contradictions sit open; the resolver exists per the roadmap. Run it on the sleep cycle to resolve/mark beliefs and set `valid_until`+`superseded_by` on losers. | D1 | `sleep/conflict_resolver.rs`; `kg_beliefs` writer | ~60 lines |
| **P16** | **Bi-temporal symmetry on the KG side.** Set `valid_from` on entity creation; set `valid_until`/invalidation on relationships when superseded; let belief supersession write `valid_until`. | D2 | `services/knowledge-graph/src/*`; entity/relationship/belief writers | ~40 lines |
| **P17** | **Inter-cluster relationship synthesis (H-4 on relationships).** Run the hierarchy builder's relationship half so `is_inter_cluster=1` edges are created between cluster-representative entities. | D3 | `sleep/hierarchy_builder.rs` | ~80 lines |
| **P18** | **Persist `kg_episode_payloads`.** The episode→KG path marks 70 episodes "done" with 0 bytes; persist the source text so downstream extractors have input. | D4 | episode ingestion path | ~30 lines |
| **P19** | **Read-side access tracking + decay-by-use.** Add `access_count`/`last_accessed_at` to `memory_facts`/`kg_beliefs`/`procedures` (they exist on entities/relationships but are always 0); bump on recall; feed into ranking/decay so unused knowledge ages out and frequently-relied-upon knowledge is prioritized. | D5 | store layer + `recall/mod.rs` ranker | ~80 lines + migration |
| **P20** | **Refresh `skill_index_state` on a schedule** (stale 7 days) + populate `kg_goals`/`kg_causal_edges` if those subsystems are intended to be live (else document as intentionally deferred). | D6 | indexer; sleep cycle | ~30 lines |

### Tier 5 — Reliability / data-quality (small)

| # | Proposal | Closes | Files |
|---|---|---|---|
| **P21** | Normalize entity-type casing (`Concept`/`concept`) at extraction. | D7 | `services/knowledge-graph/src/extractor.rs` |
| **P22** | Disambiguate `correction` as category vs `epistemic_class` in the fact writer. | D7 | fact writer |
| **P23** | Fix the recurring `multimodal_analyze` 400 (4/11 invocations). | C5 | `runtime/agent-tools/src/tools/multimodal.rs` |
| **P24** | Fix `kg_episodes` lifecycle (`started_at` always NULL; skips running→done). | D4 | episode writer |

---

## 5. Recommended sequencing (max impact first)

1. **Sprint 1 — Make application reliable & observable (Tier 1).** P1 (provenance) + P5 (soft min_score floor) + P2 (recall_log) + P3 (QueryGate) + P4 (goals). These are the cheapest, highest-leverage fixes for "hit and miss" and make every later change *measurable* via recall_log. ~1–2 days.
2. **Sprint 2 — Close the procedural loop (Tier 2, P7+P8+P11 first).** This is the direct fix for "same tool failures resurface." P7 (failure writer) + P8 (pre-tool recall) + P11 (live reinforcement) form the closed loop; P9/P10 widen the intake. ~3–4 days.
3. **Sprint 3 — Stop the context balloon (Tier 3).** P12 (budgeted recall) + P13 (plan-on-demand) directly fix "plans read upfront" and the 1M–11M-token sessions. ~3–4 days.
4. **Sprint 4 — Reconcile accumulated knowledge (Tier 4).** P15 (belief resolution) + P19 (read-side tracking) turn write-only stores into closed loops; P16/P17/P18 complete bi-temporal + hierarchy. ~5–7 days.

**Quick wins to ship today:** P1, P2, P21, P23, P24 — all small, all additive, all remove real friction.

---

## 6. Confidence & caveats

- **Verified directly:** A1 (`min_score=0.3` + override test), A2 (provenance not rendered), A7 (recall_log 0 rows + no-op trait default + existing SQLite impl), all DB population/recency numbers (queries live in analysis session), tool-failure counts (execution_logs).
- **Inferred from code, not a live settings file:** production `RecallConfig` values may override `min_score` and `mid_session_recall.enabled`. If `min_score` is already lowered in `settings.json`, A1's impact shrinks — **verify against the live vault config before sizing P5.**
- **Data window:** conversations.db holds only 7 days (2026-06-22 → 06-29), not 30. All "recent" numbers reflect one week.
- **`messages.tool_results` is uniformly NULL** in conversations.db — tool outcomes live in `execution_logs(category='tool_result')`. Any future tool-failure analytics must read execution_logs, not messages.
- **Not assessed here:** multi-agent/federation memory (Pattern 4, out of scope for this single-agent analysis); the cosine-calibration of `procedures_index` score floors.

---

*Cross-references: [[project_belief_network]] · [[project_bitemporal_memory]] · [[project_hierarchical_memory_plan]] · [[project_reflective_memory_roadmap]] · [[project_memory_crate_extraction]] · [[feedback_memory_docs_keep_in_sync]]*
