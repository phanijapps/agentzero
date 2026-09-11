# Code Intelligence + Agent Research — 2026-09-11

Two parallel tracks: engram codegraph audit (ground-truthed by rg) and
autonomous-agent research mapped to zbot's actual mechanisms. Full reports in
this directory.

## Code intelligence verdict

The graph's 15,109 "dead" count is **not actionable** — systematic
misclassification (same-file usage, feature gates, test containers, stale
symbols from removed code). Ground truth after audit:

- 1 symbol deleted (usage-proven): A2uiCapabilities
- gateway-a2a family = live feature surface (--a2a-gated), retirement is a product decision
- **Flagged: services/daily-sessions `generate_summary` has zero callers** — dead crate candidate
- The graph itself needs the code-liveness rebuild (mem-alpha 5fe733e) for trustworthy verdicts

Bridges (change-amplification hotspots, from architecture op): ExecutionRunner::finish_initial_invoke, InvokeBootstrap::finish_setup, ProgressPolicy::prepare, recall_unified_outcome_with_visibility — expected, these are the orchestration seams.

## Agent research — where zbot stands

**At/near SOTA (file-cited in agent-research.md)**: memory ranking (relevance × recency × usage via engram fusion, 76.7% precision@5), failure feedback (structured nudges + failed-episode avoid-list), evidence-driven tool diet, subagent isolation, checkpointed recovery.

**Confirmed absent (grep-verified)**:
- importance scoring — the Generative-Agents third term; no hits anywhere
- in-session reflexion persistence
- step-level plan verification
- `DelegationRequest.parallel` declared, never read (zero production reads)

## TOP-5 next improvements (impact/effort ranked)

1. **Task-level golden runs** (M) — end-to-end session assertions; would have caught both live bugs this week (ward race, procedure contract)
2. **Importance scoring** (S) — one schema field + multiplier in the fusion; golden-set measurable; completes the recency×relevance×importance triple
3. **Capability preamble** (S) — kills the 25 lookup_capabilities turns (models asking "what can I do?" at session start)
4. **In-session reflexion store** (S) — discharge-on-success writes a correction/pattern fact mid-session, not only at distillation
5. **Wire the parallel flag** (M) — gated on #1 landing first
