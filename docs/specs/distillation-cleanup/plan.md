# Plan: Distillation Cleanup — Apply Intent Analysis Lessons

## Current state

`gateway/gateway-execution/src/distillation.rs` — 3,053 lines (2,287 prod + 766 test).
Already decomposed the `distill()` god-method into phases (T6 wave). The LLM call path
uses plain `client.chat()` (NOT `prompt_typed`) — this is already the right approach
per Lesson 2. The parsing uses `parse_distillation_response()` which extracts JSON
from text — same pattern that works for intent analysis.

## What's already correct (no changes needed)

| Aspect | Current | Lesson applied? |
|---|---|---|
| LLM call | `client.chat()` — plain text, no `response_format` | ✓ Lesson 2 |
| Output parsing | `parse_distillation_response()` extracts JSON from text | ✓ Lesson 1 |
| Provider fallback | Tries target → default → first provider | Reasonable (different from retry cascade) |
| Fail-soft | Distillation failure logs warning, doesn't crash session | ✓ Lesson 8 |

## What needs cleanup

### T1: Delete dead types (Lesson 6)

Audit for zero-consumer types:
- `ExtractedFact` → consumed by `upsert_distilled_fact` — KEEP
- `ExtractedEntity` → consumed by graph projection — KEEP  
- `ExtractedRelationship` → consumed by `project_distilled_graph` — KEEP
- `ExtractedEpisode` → consumed by episode creation — KEEP
- `ExtractedProcedure` → consumed by procedure upsert — KEEP
- `ProcedureStep` → consumed by procedure upsert — KEEP
- `DistillationResponse` → consumed by parse — KEEP
- `GraphProjectionOutcome` → check if consumed post-decomposition

All types have live consumers — no dead types found. Lesson 6 already applied by the T6 decomposition.

### T2: Simplify the provider selection (Lesson 5)

`build_llm_client()` tries multiple providers in a loop. This is NOT a retry
cascade (it's trying different providers, not retrying the same one) — it's
provider fallback. But the loop is ~100 lines and could be simplified.

Current: target → default → first (with per-provider error tracking)
Simplified: try target (if configured), then default. Log and fail if both fail.

### T3: The distill() orchestrator is already clean

The T6 decomposition left `distill()` as an 81-line orchestrator calling:
- `load_session_transcript` (34 lines)
- `upsert_facts_with_dedup` (141 lines)  
- `project_knowledge_graph` (23 lines)
- `store_episode_and_procedure` (44 lines)
- `compile_ward_wiki_best_effort` (47 lines)

This is the right shape. No changes needed.

### T4: The wiki compilation is a separate concern

`compile_ward_wiki` uses its own LLM call. It's called from distillation but
is really a separate subsystem. It should move to its own module (or stay
as a focused helper). Not urgent — it works.

## What we do NOT change

1. **Don't give the distiller agent tools** — distillation reads a transcript
   (already in memory), it doesn't need to search. Lesson 3 applies to intent
   (search-driven) not to distillation (transcript-driven).

2. **Don't change the extraction schema** — the ExtractedFact/Entity/Relationship
   types are the contract with the LLM prompt and the store consumers. Changing
   them breaks the wiki, graph projection, and fact dedup.

3. **Don't change the graph projection** — the canonicalize/governance logic
   is complex but correct. It's post-LLM validation of graph data (not the
   smell from Lesson 4 — it's structural validation, not existence checks).

## Tasks

| Task | What | Effort |
|---|---|---|
| T1 | Verify no dead types (done — none found) | ✅ |
| T2 | Simplify `build_llm_client` provider loop | 30 min |
| T3 | Verify `distill()` orchestrator is clean (done) | ✅ |
| T4 | Move `compile_ward_wiki` to focused module | Optional |

## Expected outcome

Distillation is already in better shape than intent analysis was. The main
cleanup is simplifying the provider selection. No architectural change needed.
