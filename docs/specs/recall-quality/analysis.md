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

## 4. Design — make the ranking real, then make patterns actionable

### Phase 1 — Un-dark the scoring engine (facts path)

All applied uniformly in **one post-fetch scorer** (kills F1/F2/F4/F6's cause):

1. **Carry legacy signals into unified**, per fact item after store fetch:
   `final = base × decay(age, half_life[category]) × log2(2 + usage) ×
   (contradicted ? 0.5 : 1)` — pinned/skill/agent exempt from decay (rules
   already exist, `mod.rs:570-573`). Re-sort the list after scoring.
2. **Weighted RRF**: `w_source/(k + r)` with per-source weights (facts 1.0,
   procedures 1.1, beliefs 0.9, graph 0.8, wiki 0.9; defaults, tunable).
   Preserve normalized relevance inside the weight when sources expose
   comparable scores (facts do; graph confidence already multiplies).
3. **Access reinforcement**: on a recall hit that survives into the final
   packet, bump `mention_count` and set `last_accessed` (new column;
   `updated_at` keeps meaning "content changed"). One `UPDATE … WHERE id IN
   (…)` per recall — negligible cost. This is the ACT-R term: usage, not
   just re-derivation, strengthens memory.
4. **Honest telemetry**: `ranking_reasons` reflects what actually executed;
   add `weighted_rrf`, `temporal_decay`, `usage_boost` when enabled.
5. **Enable MMR by default** (λ=0.6, pool 30) — it exists, is tested, and
   diversity is cheap insurance against near-duplicate fact floods
   (3,079 domain facts).

### Phase 2 — Patterns that plan (the real ask)

6. **Procedure score = similarity × track-record**: `(1 + success_count) /
   (2 + success_count + failure_count)` multiplier, decayed by `last_used`
   age. The fields exist; recall never reads them. Render the record in the
   content: "Procedure: research_and_visualize (used 12×, 11 ok)".
7. **Planner link — pattern handoff**: when the intent agent runs (it has
   MemorySearchTool), search procedures by the task summary; top match
   above threshold enters the intent output as
   `suggested_procedure: {name, match, record}`. The plan step then either
   binds `run_procedure(name, args)` or explicitly deviates. Patterns stop
   being reading material and become **plan candidates**.
8. **Episode learnings → planner**: failed-episode avoid-list (shipped)
   joins the intent input, not just session bootstrap, so plans avoid
   known-dead approaches at construction time.

### Phase 3 — Structure (enables 1–8 cheaply)

9. Split the monolith along the source seam:
   `trait RecallSource { fn fetch(&self, qctx) -> BoxFuture<Vec<ScoredItem>> }`,
   one impl per source (≈60–90 lines each), a `SourceRegistry`, and one
   `score_and_fuse()` that owns decay/boost/weighted-RRF/MMR. The 676-line
   function becomes a 60-line pipeline. Golden-trace the before/after.

### Evaluation (no vibing)

- **Golden recall set**: 30 real queries sampled from session history with
  hand-labeled "should-have-recalled" facts/procedures.
- Metrics: hit@5, correction-recall@5 (user corrections must surface —
  they're the highest-value class), pattern-recall@5, stale-fact rate
  (superseded-but-similar facts in top 5 — should go to ~0), latency Δ.
- A/B the scoring stages on the same set: rank-only RRF (today) vs
  +weighted vs +decay/usage vs +MMR — each stage must earn its place.

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
