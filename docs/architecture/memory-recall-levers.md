# Memory Recall — Tunable Levers

Reference for the config knobs that control recall behavior. All live in `RecallConfig` (`gateway/gateway-memory/src/lib.rs`) and are **overridable** — partial configs deep-merge onto `Default` (e.g. `recall_config.json`). Tune by observation; defaults are conservative.

> Philosophy: every behavior-changing recall fix is gated behind a lever so the precision/recall tradeoff is adjustable without code changes.

## Score thresholds (what surfaces)

| Lever | Default | Effect | Slice |
|---|---|---|---|
| `min_score` | `0.3` | Hard floor for *confident* facts — items ≥ this always enter fusion. | existing |
| `low_conf_floor` | `0.1` | **Soft floor** — borderline facts in `[low_conf_floor, min_score)` surface as a small low-confidence tail instead of being silently dropped (SQLite-vec routinely scores relevant facts 0.1–0.3). | slice 5 (min_score soft floor) |
| `low_conf_tail` | `3` | Max borderline facts kept per recall. Bounds noise — the tail cap is the precision guard. Set `0` to disable the soft floor (revert to hard-drop behavior). | slice 5 |
| `graph_traversal.min_kg_confidence` | `0.1` | KG entity/edge confidence floor for graph recall. | existing |
| `high_confidence_threshold` | `0.9` | Above this = "high confidence" (used in tiering/weighting). | existing |
| `contradiction_penalty` | `0.7` | Multiplier applied to contradicted facts' scores. | existing |

## Recall rendering (what the agent sees)

| Lever | Default | Effect | Slice |
|---|---|---|---|
| confidence rendered inline | on | Non-belief items render `- [fact 0.92] content` so the agent can weight strong vs borderline recall. (Hardcoded formatter, not a config flag — see `format_scored_items`.) | slice 3 (confidence rendering) |
| `recall_log` | on (wired) | Per-session log of surfaced fact ids → `conversations.recall_log`. Observability + basis for predictive recall. | slice 4 (recall_log wiring) |

## Fusion / ranking

| Lever | Default | Effect |
|---|---|---|
| `category_weights` | schema 1.6, belief/correction 1.5, strategy 1.4, user 1.3, … | Per-category score multipliers before RRF fusion. |
| `ward_affinity_boost` | — | Boost for facts scoped to the active ward. |
| `mmr.enabled` / `lambda` / `candidate_pool` | — | MMR diversity rerank. When off, RRF output is byte-identical to pre-MMR. |
| `max_facts` / `max_episodes` | 10 / 3 | Per-subsystem caps feeding fusion. |

## Cadence

| Lever | Default | Effect |
|---|---|---|
| `mid_session_recall.every_n_turns` | `5` | Re-run legacy gated recall every N turns. |
| `mid_session_recall.min_novelty_score` | `0.3` | Mid-session items below this are skipped (novelty gate). |

## Context composition (Spec 3)

| Lever | Default | Effect | Slice |
|---|---|---|---|
| plan compaction | on | Continuation message inlines a compact step-outline (not verbatim `plan.md`); full plan fetched on-demand via `ctx.<sid>.plan`. | slice 6 (plan compaction) |
| `compaction_cap` (context-editing trigger) | chat `32K` / deep `64K` | Tool-result compaction fires when context exceeds this (not only at 70-80% of the window, which large windows never reach). **The main token-bloat lever** — tune up to retain more history, down to compact harder. | slice 7 (tool-result compaction cap) |

## Not-yet-wired levers (gaps → future slices)

| Lever | Status | Fix slice |
|---|---|---|
| QueryGate routing on `recall_unified` | **bypassed** (gate exists, wired only to legacy `recall()`) | Spec 1 (P3) — route by query type so "thanks!" doesn't hit all 9 stores |
| `active_goals` → `intent_boost` | **dead at bootstrap** (passed `&[]`) | Spec 1 (P4) — thread goals to prioritize goal-aligning recall |
| Budgeted recall (token) | **missing** — recall injected with no capacity budget against the window | Spec 3 (P12) — budget recall so it never pushes the prompt toward compaction |

---
*Cross-references: gap analysis (`memory-bank/future-state/2026-06-28-memory-application-gap-analysis.md`) · modularization plan (`…2026-06-29-memory-layer-modularization-plan.md`) · slice specs (`docs/specs/recall-*`, `docs/specs/kg-*`).*
