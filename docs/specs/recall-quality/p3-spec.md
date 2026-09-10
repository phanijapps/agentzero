# P3 Spec: sleep → engram consolidation (port-slot architecture)

## What investigation found (read before touching anything)

The 10K wholesale-deletion estimate assumed engram's executors replace zbot's
modules outright. Reality is finer — and better structured:

**Engram's consolidation is a composite of `ConsolidationMutationExecutor`
impls driven by `ConsolidationRequest` → plan → run → auditable results.**
Deterministic executors exist (contradiction detection via the
`ContradictionDetector` port, lifecycle auto-archive that is **usage-aware** —
archives by last-retrieval age, pairing with P1b's `touch_facts`). LLM work is
deliberately deferred behind **trait slots**:

| engram port | slot status | zbot module that fills it |
|---|---|---|
| `BeliefSynthesizer` (engram-belief) | deterministic baseline only; "real LLM impl replaces this behind the same trait" | sleep/belief_synthesizer.rs LLM core |
| `HierarchyBuilder` (engram-hierarchy) | "may use clustering, taxonomy, graph structure, or model-assisted summaries internally" | sleep/hierarchy_builder.rs + llm_aggregate_entity.rs |
| `ContradictionDetector` (engram-belief) | port; executor wires it | sleep/belief_contradiction_detector.rs (deterministic core) |
| `ProcedureRepository` (engram-procedures) | storage port; extraction is host-side | sleep/pattern_extractor.rs LLM core |

## Target shape

```
zbot sleep worker (thin, ~150 lines)
  └─ engram ConsolidationRequest (task kinds: BeliefSynthesis,
     BeliefContradictionDetection, BeliefPropagation, HierarchyBuild,
     Decay, Pruning, Compaction, OrphanArchival, ProcedureExtraction)
       ├─ engram deterministic executors (lifecycle/contradiction/decay)
       └─ zbot port impls (LLM collaborators + zbot-store bridges)
```

## Revised deletion estimate

LLM cores survive as port impls; orchestration + store plumbing + worker
scheduling die. Net: **~10,400 → ~4,900 surviving**, ≈ **5,500 deleted**
(plus the worker.rs hand-rolled scheduling → ~150-line trigger).

## Sub-batches (each gated: workspace check/clippy/fmt + gateway-memory tests
+ gateway-execution test-stubs + golden recall floors)

1. **Belief cluster** (~2,500): belief_synthesizer → `BeliefSynthesizer` impl
   (LLM core kept, scheduling/dedup plumbing deleted — engram's executor owns
   persistence via `BeliefRepository`, which the adapter already uses);
   belief_contradiction_detector → `ContradictionDetector` impl;
   belief_propagator → deleted if engram's propagation covers it (verify
   core/belief propagation surface; else port impl).
2. **Hierarchy + decay cluster** (~3,300): hierarchy_builder +
   llm_aggregate_entity → `HierarchyBuilder` impl; decay/pruner/compactor/
   orphan_archiver → deleted where engram's Decay task + MemoryLifecycle
   Executor cover (verify against zbot's per-category half-lives — the decay
   knobs move into the trigger config).
3. **Extraction + remaining** (~1,600): pattern_extractor → host executor
   registered under ProcedureExtraction (uses engram `ProcedureRepository`
   via adapter; success/failure counters → `increment_n`); synthesizer/
   clustering/corrections_abstractor disposition (corrections_abstractor
   moves to distillation per plan); worker.rs → consolidation trigger.

## Invariants

- Golden recall floors: 30/30 presence, tag floors, ward isolation, pinned
  survival, superseded suppression (sleep writes must not regress recall).
- `memory_events` / consolidation runs become the audit trail (replaces
  zbot's ad-hoc tracing in deleted modules).
- Wiki untouched. KG untouched (separate decision).
