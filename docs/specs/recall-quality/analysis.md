# Recall: Critical Analysis & Design

**Date:** 2026-09-09 · **Scope:** the production recall path (`recall` tool →
`recall_unified_outcome_with_visibility`), the scoring engines, and the
pattern-surfacing loop (procedures/episodes).

**Verdict up front:** recall's *plumbing* is genuinely strong — eleven fused
sources, provenance-scoped visibility, supersession filtering, taxonomy
expansion, an MMR stage. But its *ranking* is hollow. The sophisticated
scoring engine that the codebase already contains (temporal decay, usage
reinforcement, contradiction penalties) runs only on a legacy path the model
no longer calls. The production path ranks by raw position in a rank-fusion
that discards every calibrated signal, several advertised features are
provably no-ops, and the agent's actual usage of memory feeds back into
nothing. This is a memory layer that *collects* well and *retrieves* naively.

---

## 1. What the production path actually does today

`recall` tool → `GatewayUnifiedRecallAdapter::recall` → `recall_unified_outcome_with_visibility` (`recall/mod.rs:694`, 676 lines):

1. Taxonomy (SKOS) query expansion — scope-proven, optional
2. Query embedding (+ identity tracking for provider drift)
3. Eleven sources fan out: facts (hybrid FTS+vector), wiki, procedures, graph
   ANN, graph traversal, previous episodes, active goals, beliefs, hierarchy
   LCA, hierarchy relations, profile atoms
4. Per-source projection into `ScoredItem`, scope-visibility filtering
5. `intent_boost` on each list
6. `rrf_merge(lists, k=60)` — reciprocal rank fusion
7. Optional MMR diversity rerank (**disabled by default**, `lib.rs:1077`)
8. Budget truncate → context packet render

## 2. Critical findings (severity-ordered)

### F1 — Two scoring engines; the good one is dark

The legacy `recall()` (`mod.rs:456`) has a *complete* scoring pipeline:

| Signal | Code | Status |
|---|---|---|
| Per-category temporal decay, hyperbolic `1/(1+age/half_life)` | `mod.rs:568-586` | **legacy only** |
| Usage reinforcement: `log2(mention_count)` boost | `mod.rs:587` | **legacy only** |
| Contradiction penalty | `mod.rs:591-595` | **legacy only** |
| Ward affinity boost | `mod.rs:560-563` | **legacy only** |
| Self-RAG query gate (Skip/Direct/Split) | `query_gate.rs`, `mod.rs:463-470` | **legacy only, opt-in** |
| Category-aware supersession penalty | `mod.rs:600+` | **legacy only** |

The unified path — the only one the model can reach since the tool-surface
consolidation — applies **none of these**. The design was built, tested,
documented… and never carried across.

### F2 — `intent_boost` and category weights are provably no-ops in unified

`rrf_merge` scores by **vector position** (`enumerate()`), not by `item.score`
— and the fused score *overwrites* `item.score`:

```rust
// scored_item.rs:65 — rank comes from position, scores are discarded
for (rank_zero, item) in list.into_iter().enumerate() { ... }
item.score = fused_score;   // ← source score destroyed
```

`intent_boost` multiplies `item.score` in place (`mod.rs:1336-1338`) but the
lists are **never re-sorted** before fusion — so the multiplier changes
nothing. Same for the uniform per-source category weights (belief 0.9,
pattern 0.9): uniform scaling within a list preserves order → zero effect.
Worse, the telemetry *advertises* `intent_boost` as an active ranking reason
(`mod.rs:1307`) — **the observability layer lies about what ran**.

### F3 — RRF's one real strength never fires

RRF's power is cross-list agreement: the same item in multiple lists sums
contributions. But every source uses a disjoint id namespace (`fact:<uuid>`
vs `graph:<name>` vs procedure ids) — **agreement is impossible by
construction**. What remains is rank-interleaving with k=60 flattening, and
all calibrated relevance (cosine 0.95 vs 0.35, entity-confidence products,
hop decay) is discarded at the fusion seam.

### F4 — `updated_at`: written everywhere, read nowhere

Production store (`engram memory_facts`, 7,147 rows): `updated_at`,
`mention_count`, `valid_from/valid_until` all persisted. Ranking use: none.
The reinforcement loop is inert in practice — **415 of 7,147 facts have
`mention_count > 1`** (mostly ctx facts from save repetition, avg 4.27; the
durable categories average ~1.00). Recall access bumps nothing: the agent
using a fact does not strengthen it. Distillation re-mention bumps it, but
only if a later session re-derives it verbatim.

### F5 — Patterns are surfaced, never *planned with*

798 procedures exist. `Procedure` carries `success_count`/`failure_count`/
`last_used` — and the top-used procedure in production has **2 uses**. The
counters barely fire, and recall ignores them: a 12/1 procedure and a 0/5
procedure rank identically at the same similarity. Recall renders procedures
into the *resource lane* beside wiki articles ("Procedure: name, Steps: N")
— nothing tells the planner "this is an executable, proven pattern; adapt it
or `run_procedure` it." `run_procedure` (with step interpolation) exists and
is registered, but the linkage recall→plan is a coincidence of the model
reading the resource lane and remembering the tool exists. Episodes (chain
continuity, now with the avoid-list) inject as prose; strategies have a
separate supersede path — but none of this reaches the *planning* step as
input.

### F6 — The 676-line function is monolithic because the seam is wrong

Sources are already abstracted as adapter *functions*, but orchestration,
telemetry, visibility, and fusion are one body. The natural seam — a
`RecallSource` trait (`fetch(&QueryCtx) -> Vec<ScoredItem>`) with a uniform
post-fetch scorer — would make each source one impl and the scoring pipeline
one place. Instead every source's glue is inline, which is *why* F1's
signals never got carried: there is no single place to apply them.

## 3. Industry grounding

- **Generative Agents (Park et al., 2023)** — the canonical memory retrieval
  score is `recency × importance × relevance`. Recency: exponential decay
  per access. Importance: LLM-scored at write time. zbot has relevance only
  (and even that is rank-flattened). Of the triple, **two of three are
  missing in production**.
- **ACT-R base-level activation (Anderson & Schooler)** — memory activation
  `B = ln(Σ t_j^-d)`, `d ≈ 0.5`: strength from **frequency and recency of
  accesses**, each access slows decay. This is the theoretical basis for
  access-reinforcement: the score should grow when the *agent uses* the
  memory, not only when distillation re-derives it. zbot's `mention_count`
  was designed for exactly this and never wired to recall.
- **Power law of forgetting (Ebbinghaus; Wixted)** — retention decays as a
  power function, not exponential. The existing `1/(1 + age/half_life)`
  hyperbola is a power law in disguise — **the legacy formula was right**;
  it just never ran in production.
- **RRF (Cormack et al., 2009)** — designed to fuse *independent rankers
  over the same corpus*; the agreement term is the point. For disjoint
  corpora, **weighted RRF** (`w_source × 1/(k + r)`) is the standard
  adaptation, and preserving per-source score distributions (or normalizing
  them into the weight) is the known requirement.
- **Self-RAG (Asai et al., 2023)** — retrieval gating (should we retrieve?
  split multi-topic queries) reduces dilution. zbot *has* a full gate
  (`query_gate.rs`, Skip/Direct/Split, fail-safe) — wired only into the
  dark path.
- **Mem0 (2025) / MemGPT-Letta** — write-time consolidation (ADD/UPDATE/
  DELETE/NOOP decisions; zbot's supersede/dedup covers this) plus
  retrieval that is embedding similarity **with recency weighting**; MemGPT
  adds self-editing memory. Industry consensus: retrieval without a time
  term is considered incomplete.
- **Time-weighted retrieval in practice** (LlamaIndex `TimeWeightedRetriever`):
  `score = semantic_similarity + (1 - decay_rate)^hours_since_access` — a
  post-hoc multiplier on similarity, i.e. exactly the shape of the fix
  below.

## 4. The cleaner solution — use what engram already baked

**Revised after auditing the engram repo** (`~/projects/mem-alpha` = the
`phanijapps/engram` the adapter already depends on). Engram's retrieval core
already implements — tested, contract-frozen — almost everything my first
design proposed to build in gateway-memory:

| My Phase-1 proposal | Engram already has |
|---|---|
| Weighted RRF (`w_source/(k+r)`) | `ReciprocalRankFusion` + `ReciprocalFusionConfig { k, default_source_weight, source_weights }` (`core/retrieval`) |
| Recency decay on facts | `TemporalRetrievalIndex` + `recency_score` exponential decay, half-life 14d (`adapters/sqlite/src/memory/temporal_retrieval.rs`) |
| Multi-factor scoring | `RetrievalScore { relevance, recency, confidence, cue_match, hierarchical_fit, policy_fit }` (`core/domain/retrieval.rs`) |
| Honest telemetry | `FusionTrace` per candidate: source, source rank, source score, fusion score, dedup-with |
| MMR rerank | `RetrievalReranker` port + `adapters/retrieval/mmr-rerank` (+ cross-encoder-rerank) |
| Access reinforcement | `MemoryEvent::Retrieved` in the domain event set |
| Procedure success/failure | `Procedure { success_count, failure_count }` + `increment_n` + `ProcedureStats` (`core/procedures`) — zbot duplicated this with its own table |

**The duplication is three layers deep, and only the bottom one is good:**

1. engram core retrieval composition — *unused by zbot*
2. zbot-engram-adapter — hand-rolled hybrid SQL + its own RRF normalization over its own `memory_facts` table (bypasses engram's memory service)
3. gateway-memory unified — its own `rrf_merge`, its own `mmr.rs`, its own (dead) legacy scoring

### The plan: make the adapter an engram retrieval citizen (no data migration)

**Phase 1 — Route the memory lane through engram's composer (2–3 days)**

1. zbot-engram-adapter implements engram's `RetrievalIndex` port for the fact
   store: the existing hybrid SQL becomes the `fact-hybrid` lane, returning
   `RetrievalResult`s with `score.relevance` set (and `confidence` from the
   fact's own field).
2. Add a temporal lane over the same table using engram's `recency_score`
   (per-category half-lives mapped from zbot's category semantics; pinned/
   skill/agent exempt). Recency enters the fused score as a first-class
   factor — not a gateway-side bolt-on.
3. Fuse with engram's `ReciprocalRankFusion` + per-source weights. The
   adapter's `normalize_rrf_score` is deleted; gateway-memory's `rrf_merge`
   is deleted for the memory lane (kept only for zbot-policy lanes until
   Phase 3).
4. Reinforcement: on final-packet hits, write `MemoryEvent::Retrieved` (the
   event kind already exists in the domain) and bump `mention_count` +
   `last_accessed`. One batched UPDATE per recall.
5. Honest telemetry for free: `FusionTrace` replaces the fabricated
   `ranking_reasons` list.
6. MMR via engram's `mmr-rerank` adapter, enabled by default; gateway-memory's
   own `mmr.rs` deleted.

**Phase 2 — Procedures converge on engram's procedural memory (1–2 days)**

7. `run_procedure` increments engram's `increment_n` (success/failure
   accounting is the port's own contract); zbot's parallel counters die.
8. Procedures lane scores with track record: `similarity × (1+success)/(2+
   success+failure)` × recency-of-last-use, rendered in content
   ("used 12×, 11 ok"). Fields already exist on both sides.
9. Planner handoff (unchanged from prior design): top procedure above
   threshold enters the intent agent's output as `suggested_procedure`
   via its MemorySearchTool query — patterns become plan candidates.
   Failed-episode avoid-list joins the intent input the same way.

**Phase 3 — Collapse the gateway unified function (1.5 days)**

10. Split `recall_unified_outcome_with_visibility` along the same seam
    engram's ports already define: each zbot-policy source (goals,
    episodes+avoid, profile, wiki) becomes a `RetrievalIndex` impl; the
    unified function shrinks to registry → fuse (engram) → zbot visibility →
    render. The dead legacy scoring block and both hand-rolled RRFs are
    deleted, not carried.

**Later (explicitly not now):** migrating zbot's `memory_facts` table into
engram's `memories` MemoryRecord store — real but a data migration
(7,147 rows, field mapping, conformance-parity run). Phase 1 gets the
ranking wins without it; revisit when engram's consolidation pipeline
(decay/pruning tasks) is something zbot wants to run in its sleep cycle —
that's the moment full citizenship pays.

### Evaluation (unchanged, still no vibing)

Golden recall set: 30 real queries from session history, hand-labeled.
Metrics: hit@5, correction-recall@5, pattern-recall@5, stale-fact rate
(superseded-but-similar in top 5 → ~0), FusionTrace-explainable share
(should be 100%), latency Δ vs the three-RRF stack.

## 5. What NOT to do

- **No learned reranker yet** — a cross-encoder needs labeled pairs and a
  serving path; the score-carried fixes above capture most of the value at
  ~zero cost. Revisit only if golden-set headroom remains.
- **No embedding-model change** — orthogonal; the pipeline should stabilize
  first (query-identity tracking already handles drift).
- **No new memory kinds** — the schema (facts, procedures, episodes,
  beliefs) is sound; this is a retrieval-quality problem, not a storage
  problem.

## 6. Effort

| Phase | Estimate |
|---|---|
| P1 (signals + weighted RRF + reinforcement + honest telemetry + MMR default) | 2–3 days incl. golden set |
| P2 (procedure record scoring + planner handoff) | 2 days |
| P3 (source-trait split) | 1.5 days, mechanical after P1 lands |

P1 first — it is the highest ratio of agent-quality-per-line in the repo,
and P2's pattern handoff is only meaningful once the ranking beneath it is
real.
