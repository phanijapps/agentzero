# Belief Network Design

**Date:** 2026-05-15
**Status:** Northstar — design ready, implementation queued
**Context:** Final phase (Phase 4) of the reflective memory roadmap. Bi-temporal wiring (PRs #143-146, design at `2026-05-15-bitemporal-wiring-design.md`) was the explicit prerequisite.
**Related:**
- `[[project_reflective_memory_roadmap]]` — this is the last unbuilt phase
- `[[project_bitemporal_memory]]` — beliefs build on truth-intervals
- `[[project_memory_crate_extraction]]` — beliefs should land generic from day one (use `partition_id`, not `ward_id`)
- `[[2026-05-15-memory-crate-genericness-audit]]` — the audit's recommendations apply to any new code

---

## What This Enables

Three capabilities the memory layer does not have today:

1. **Multi-fact aggregation.** Today the agent has facts. Asked "what do you believe about the user's job?" it gets back N raw facts and has to synthesize on the fly. After this: it gets a *belief* — a synthesized stance with its own confidence, source provenance, and truth-interval.

2. **Contradiction graph.** Today ConflictResolver does pairwise supersession — A loses to B, A.superseded_by = B.id. After this: contradictions are first-class entities with type (logical/tension/temporal), severity, resolution status, and provenance. The system can answer "what do we have conflicting evidence about?" — not just "what did we already resolve?"

3. **Confidence propagation.** Today if a fact loses confidence (e.g., user retracts it), other facts/beliefs derived from it stay at their original confidence. After this: invalidation cascades through the belief graph. Beliefs built on retracted evidence weaken proportionally.

Together these turn "a pile of facts" into "a reasoned stance the agent maintains."

---

## Vocabulary

### Fact vs. Belief

| | Fact | Belief |
|---|---|---|
| **Atomicity** | Atomic — one claim, one source episode | Aggregate — synthesized from one or more facts |
| **Confidence source** | Set at write time by the writer (LLM judge, user statement, etc.) | Derived from constituent fact confidences + recency + cross-validation |
| **Lifecycle** | Created once, may be superseded or decay | Re-synthesized as supporting facts change; propagation rules apply |
| **Storage** | `memory_facts` | `kg_beliefs` (new) |
| **Identity** | `fact_id` | `belief_id`, keyed by `(partition_id, subject, valid_from)` |
| **Example** | "User said 'I work at OpenAI' on 2026-04-01" | "User works at OpenAI (since 2026-04-01)" — synthesized from one or more user-statement facts |

### Contradiction vs. Supersession vs. Tension

| Type | Semantic | Handled by |
|---|---|---|
| **Supersession** | Temporal — A was true until B replaced it | Existing bi-temporal: A.valid_until = B.created_at, A.superseded_by = B.id |
| **Contradiction** | Logical — A and B cannot both be true | NEW: `kg_belief_contradictions` row, type="logical" |
| **Tension** | A and B don't strictly contradict but suggest different things | NEW: `kg_belief_contradictions` row, type="tension" |

Crucially: not every contradiction is a supersession. "User prefers dark mode" and "User prefers light mode" *might* be a supersession (user changed mind) or *might* be tension (user switches based on context). The Belief Network lets the agent record uncertainty about which.

---

## Schema

Two new tables. Both partition-scoped from day one using `partition_id` (the audit's R-series target for `ward_id` — beliefs are added generic so they don't need a future rename).

### `kg_beliefs`

```sql
CREATE TABLE IF NOT EXISTS kg_beliefs (
    id TEXT PRIMARY KEY,
    partition_id TEXT NOT NULL,
    subject TEXT NOT NULL,            -- canonical key, e.g. "user.location", "project.x.status"
    content TEXT NOT NULL,            -- synthesized claim
    confidence REAL NOT NULL,         -- derived from constituents + recency
    valid_from TEXT,                  -- bi-temporal: when belief became true
    valid_until TEXT,                 -- when belief stopped being current
    source_fact_ids TEXT NOT NULL,    -- JSON array of constituent fact IDs
    synthesizer_version INTEGER NOT NULL DEFAULT 1,  -- which BeliefSynthesizer version produced this
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    superseded_by TEXT,
    UNIQUE(partition_id, subject, valid_from),
    FOREIGN KEY (superseded_by) REFERENCES kg_beliefs(id)
);

CREATE INDEX idx_beliefs_partition_subject ON kg_beliefs(partition_id, subject);
CREATE INDEX idx_beliefs_valid ON kg_beliefs(valid_from, valid_until);
```

### `kg_belief_contradictions`

```sql
CREATE TABLE IF NOT EXISTS kg_belief_contradictions (
    id TEXT PRIMARY KEY,
    belief_a_id TEXT NOT NULL,
    belief_b_id TEXT NOT NULL,
    contradiction_type TEXT NOT NULL,  -- 'logical' | 'tension' | 'temporal'
    severity REAL NOT NULL,            -- 0.0..1.0
    judge_reasoning TEXT,               -- LLM's explanation, for debugging
    detected_at TEXT NOT NULL,
    resolved_at TEXT,
    resolution TEXT,                    -- 'a_won' | 'b_won' | 'compatible' | 'unresolved'
    FOREIGN KEY (belief_a_id) REFERENCES kg_beliefs(id) ON DELETE CASCADE,
    FOREIGN KEY (belief_b_id) REFERENCES kg_beliefs(id) ON DELETE CASCADE,
    UNIQUE(belief_a_id, belief_b_id)
);

CREATE INDEX idx_contradictions_belief_a ON kg_belief_contradictions(belief_a_id);
CREATE INDEX idx_contradictions_belief_b ON kg_belief_contradictions(belief_b_id);
CREATE INDEX idx_contradictions_unresolved ON kg_belief_contradictions(resolved_at) WHERE resolved_at IS NULL;
```

### Why these shapes

- **`subject` is the aggregation key.** Multiple facts about `user.location` aggregate into one belief at any given moment. The subject is a canonical string — Phase 1 uses key-prefix matching (`user.*`), later phases may canonicalize via embedding similarity.
- **`source_fact_ids` is a JSON array.** Makes provenance queryable: "what facts is this belief built on?" Reading them lets the agent explain its reasoning.
- **`valid_from` / `valid_until` mirror the fact schema.** Beliefs are bi-temporal too — same point-in-time recall semantics work.
- **`synthesizer_version`** is for when the BeliefSynthesizer prompt/algorithm changes — old beliefs can be re-synthesized on demand without losing the old version.
- **Contradictions are a separate table** because they have their own lifecycle: detection, resolution, severity changes. Embedding them as belief fields would couple two concerns.

---

## Operations

### Read: `belief_query(partition_id, subject, as_of)`

Returns the active belief for a subject. Filters by `valid_from <= as_of < valid_until`. Default `as_of = Utc::now()`.

```rust
pub async fn belief_query(
    &self,
    partition_id: &str,
    subject: &str,
    as_of: Option<DateTime<Utc>>,
) -> Result<Option<Belief>, String>;
```

Behaves analogously to `recall_facts_prioritized` with `as_of` — same temporal-interval filter semantics.

### Write: `belief_synthesize(partition_id, subject)`

Sleep-time operation. Re-derives the belief from constituent facts.

```rust
pub async fn belief_synthesize(
    &self,
    partition_id: &str,
    subject: &str,
) -> Result<Belief, String>;
```

Algorithm:
1. Fetch all non-superseded facts with key matching `subject` (or `subject.*` prefix), in partition.
2. Sort by `valid_from` ascending.
3. Pick the most-recent-`valid` fact whose interval covers "now."
4. Compute belief confidence: `recency_weight(fact.valid_from) × fact.confidence` averaged across constituents (Phase 1 simple; Phase 2+ weighted by source-type).
5. UPSERT belief row keyed on `(partition_id, subject, valid_from)`. If the synthesis produces a NEW `valid_from` (winner shifted), the old belief gets `valid_until = new.valid_from, superseded_by = new.id` — same pattern as the bi-temporal conflict transition.

### Write: `belief_invalidate(fact_id, transition_time)`

Called when a fact is superseded, expires, or is explicitly retracted. Find all beliefs whose `source_fact_ids` contains `fact_id`, decide:
- If the fact was the SOLE source: belief is also retracted (`valid_until = transition_time`).
- If the fact was ONE of many sources: belief confidence drops proportional to that fact's contribution; re-synthesize.

### Write: `contradiction_detect(belief_a_id, belief_b_id)`

Sleep-time. LLM judge with a structured prompt:
- "Are these two beliefs logically contradictory, in tension, or compatible?"
- Returns `{decision: "logical"|"tension"|"compatible", severity: 0.0..1.0, reasoning: "..."}`
- On contradictory/tension: INSERT `kg_belief_contradictions` row.

### Write: `contradiction_resolve(contradiction_id, resolution)`

Mark a contradiction resolved. Resolution may cascade: if `a_won`, then `belief_b` gets `superseded_by = belief_a.id`.

---

## Sleep-Cycle Integration

New worker: **`BeliefNetwork`**. Runs after `ConflictResolver` in the existing sleep loop. Throttled by `execution.memory.beliefNetworkIntervalHours` (default 24).

Workflow per cycle:

1. **Identify dirty subjects.** Subjects with new fact activity since last cycle (track via a watermark or a `subject_last_synthesized_at` table).
2. **Re-synthesize.** For each dirty subject: `belief_synthesize(partition, subject)`.
3. **Detect contradictions.** For each freshly synthesized belief, find peers (other beliefs in the same partition with a related subject via the KG) and run `contradiction_detect`. Cap LLM calls per cycle at a configurable budget (default 20).
4. **Propagate invalidations.** For each fact whose confidence dropped since last cycle (via DecayEngine, ConflictResolver, or user retraction), find dependent beliefs and call `belief_invalidate`.

Cycle is idempotent: re-running it produces the same belief state. The watermark is the only mutable cycle-local state.

---

## Phased Rollout

The Belief Network is the largest single piece of work since the gateway-memory crate extraction. Ship in 4 phases, each its own PR:

### Phase B-1 — Belief synthesis (~3-4 days)

Minimum useful first phase. Lands the `kg_beliefs` table, the BeliefSynthesizer worker, and `belief_query`. No contradiction graph yet, no propagation.

- Migration v27: `CREATE TABLE kg_beliefs`
- `gateway-memory/src/sleep/belief_synthesizer.rs` — the worker
- `zbot-stores-traits` — `BeliefStore` trait
- `zbot-stores-sqlite` — implementation
- `MemoryServices` — wire the worker into the sleep cycle
- `runtime/agent-tools/src/tools/memory.rs` — new `belief` action on the memory tool
- Tests: synthesis from single fact, synthesis from multiple facts, re-synthesis idempotence, as_of point-in-time

### Phase B-2 — Contradiction graph (~3-4 days)

Lands `kg_belief_contradictions` + BeliefContradictionDetector. Builds on B-1.

- Migration v28: `CREATE TABLE kg_belief_contradictions`
- `gateway-memory/src/sleep/belief_contradiction_detector.rs`
- LLM judge: reuse `MemoryLlmFactory` (same path as ConflictResolver)
- Topical-neighborhood scoping via existing KG edges (avoid O(N²))
- Settings: `execution.memory.beliefContradictionBudgetPerCycle` (default 20)
- Tests: logical-contradiction detection, tension classification, neighborhood scoping correctness, budget enforcement

### Phase B-3 — Confidence propagation (~3-4 days)

Wire `belief_invalidate` into the existing fact-lifecycle events. Builds on B-1 (and optionally B-2).

- Hook into ConflictResolver's `supersede_fact` path — when a fact is superseded, find dependent beliefs and invalidate
- Hook into DecayEngine — when a fact's confidence drops below a threshold, trigger invalidation
- Cap propagation depth at 3 hops to prevent cascade explosions
- Tests: single-hop propagation, multi-hop propagation, cycle detection, threshold enforcement

### Phase B-4 — Recall integration (~1-2 days)

Recall returns beliefs alongside facts.

- `recall_unified` learns to retrieve beliefs (new `ItemKind::Belief`)
- Beliefs get a category weight in the rescore step (suggested default: 1.7 — above schema at 1.6 since beliefs are MORE distilled)
- Agent prompt block: `## Active Beliefs` formatted on the gateway side (not in gateway-memory — per the audit, presentation belongs at the consumer layer)
- Tests: beliefs surface for relevant queries, beliefs deduplicate against their source facts in results

### Phase B-5 — UI surface (~2-3 days)

Beliefs + contradictions are exposed to the operator under the existing Memory tab.

- New HTTP module `gateway/src/http/beliefs.rs` with 5 endpoints:
  - `GET /api/beliefs/:agent_id` — paginated list per partition
  - `GET /api/beliefs/:agent_id/:belief_id` — detail (belief + source-fact summaries + contradictions)
  - `GET /api/beliefs/:agent_id/:belief_id/contradictions` — full list for a single belief
  - `GET /api/contradictions/:agent_id` — recent contradictions in partition
  - `POST /api/contradictions/:contradiction_id/resolve` — body `{"resolution": "a_won"|"b_won"|"compatible"}`
- All endpoints return `503 Service Unavailable` (not `404`) when `execution.memory.beliefNetwork.enabled = false` — the route exists, the feature is dormant.
- `AppState` gains `belief_store` + `belief_contradiction_store` fields (Option-wrapped, populated only when the feature flag is on).
- React components under `apps/ui/src/features/memory/command-deck/beliefs/`: `BeliefsList`, `BeliefCard`, `BeliefDetailDrawer`, `ContradictionBadge`, `ContradictionResolver`, `ContradictionList`.
- `MemoryTab` learns a 3-way sub-tab toggle (Facts / Beliefs / Contradictions). Facts remains default; existing Facts flow is unchanged.
- Wire types live in `types.beliefs.ts`; tiny `beliefs/api.ts` does the fetches off the global transport (distinct from the main `Transport` interface to avoid surface-area churn).

**Total estimated effort: ~12-17 days across 5 PRs.**

---

## Worked Examples

### Example 1 — straightforward employment history

Inputs across time:

```
2026-01-15  fact F1: key=user.employment, content="Anthropic"
                     valid_from=2026-01-15, conf=0.9
2026-04-01  fact F2: key=user.employment, content="OpenAI"
                     valid_from=2026-04-01, conf=0.9
[ConflictResolver]   F1.valid_until=2026-04-01, F1.superseded_by=F2.id
```

After BeliefSynthesizer:

```
belief B1: subject=user.employment, content="User works at OpenAI"
           valid_from=2026-04-01, valid_until=NULL
           confidence=0.9, source_fact_ids=[F2.id]

belief B2: subject=user.employment, content="User worked at Anthropic"
           valid_from=2026-01-15, valid_until=2026-04-01
           confidence=0.9, source_fact_ids=[F1.id]
           (B2 is implicitly "historical" — its valid_until is in the past)
```

Query `belief_query("default", "user.employment", as_of=2026-02-15)` → returns B2.
Query without `as_of` → returns B1.

### Example 2 — multiple corroborating sources

Inputs:

```
fact F1 from user: "I live in Mason, OH"  conf=1.0
fact F2 from address-book sync: "user.address.city = Mason"  conf=0.8
fact F3 from old chat: "I'm moving to Mason next month"  conf=0.7
```

After BeliefSynthesizer:

```
belief B1: subject=user.location
           content="User lives in Mason, OH"
           confidence=0.95 (consensus across 3 sources outweighs any individual)
           source_fact_ids=[F1.id, F2.id, F3.id]
```

Higher confidence than any single fact because multiple independent sources agree.

### Example 3 — tension, not contradiction

Inputs:

```
fact F1: "User prefers dark mode" (from 2026-02 chat)
fact F2: "User prefers light mode" (from 2026-04 chat)
```

ConflictResolver might mark F1.superseded_by = F2 (newest wins). But that's an over-commit if the user actually switches based on context.

After BeliefContradictionDetector runs on the two beliefs:

```
contradiction C1: belief_a=B_dark, belief_b=B_light
                  type="tension"  (not "logical")
                  severity=0.4    (low — preferences can shift)
                  resolution="unresolved"
                  reasoning="User stated different preferences across time; could be
                             context-dependent (e.g., time of day) rather than a
                             true mind-change. Recommend asking next time it matters."
```

The agent now KNOWS it has conflicting evidence about display preferences and can surface the uncertainty rather than confidently assert the wrong one.

### Example 4 — confidence propagation

Inputs:

```
fact F1: "User's manager is Alice" (conf=0.9)
belief B1: subject=user.reporting_chain, content="User reports to Alice"
           source_fact_ids=[F1.id], confidence=0.9

[User retracts F1: "Actually that was wrong, Alice is in a different team"]
[ConflictResolver marks F1.contradicted_by, conf drops to 0.2]

belief_invalidate(F1.id, now()) fires:
  - B1 has F1 as its sole source
  - B1.confidence drops to 0.2 (matches single-source contribution)
  - B1 is flagged for re-synthesis next cycle
```

Without propagation, B1 would stay at conf=0.9 even though its only source is now untrusted. The agent would confidently surface a stale belief.

---

## Risks

| Risk | Severity | Mitigation |
|---|---|---|
| BeliefSynthesizer LLM cost scales with subject count | Medium | Throttle, only re-synthesize dirty subjects, cache prompts |
| Subject identification is fuzzy (`user.location` vs `user.address`) | Medium | Phase 1: exact key match. Phase 2+: canonicalize via embedding similarity (same path as schema-fact deduplication) |
| Contradiction detection is O(N²) over all belief pairs | High if global | Mitigation built in: only compare beliefs in the same topical neighborhood via existing KG edges |
| Confidence propagation could cascade indefinitely | High | Cap propagation depth at 3 hops; require minimum confidence change to trigger further propagation |
| LLM judge classifications drift across runs | Medium | Store `judge_reasoning` for every contradiction; allow user override; track judge prompt version |
| Beliefs become stale if BeliefSynthesizer cycle doesn't fire | Low | Default 24h cadence matches existing sleep workers; user can drop to 0 (every cycle) |
| `superseded_by` chain in beliefs grows long over time | Low | Same as facts — the bi-temporal model already handles supersession chains cleanly |
| Migration v27/v28 risk on production DB | Low | Both migrations are pure `CREATE TABLE` — no existing data touched. Rollback = drop tables. |

---

## Out of Scope (v1)

- **Logical inference engines.** Beliefs are aggregated stances, not first-order logic. No Prolog, no Datalog, no theorem proving.
- **Multi-hop transitive inference.** "User is at Anthropic; Anthropic is in SF; therefore user is in SF" — separate problem, not addressed here.
- **Formal belief revision** (AGM theory, Bayesian updating, etc.). The propagation rules are pragmatic, not formally grounded.
- **Cross-partition belief reconciliation.** Beliefs are partition-scoped. Reasoning across partitions ("user has different preferences in work vs. personal partition") is future work.
- **UI surfacing** of beliefs and contradictions. Belongs in `apps/ui`, separate PR.
- **Agent-driven belief assertion.** The agent can READ beliefs via the memory tool but can't directly WRITE them — they're derived from facts only. Allowing agents to assert beliefs as primary statements is future work.
- **Belief consensus across multiple agents** (federated reasoning). Pattern 4 territory.

---

## Decision Log

- **2026-05-15:** Chose to make beliefs a NEW first-class entity in their own table, not just a special kind of fact. Reason: beliefs have their own lifecycle (synthesis, propagation, contradiction) that doesn't fit cleanly into the existing `memory_facts` columns. Coupling them would force `memory_facts` to grow several conditional columns.
- **2026-05-15:** Chose `partition_id` (not `ward_id`) in the new tables. Generic from day one — when the R-series rename in the audit lands, beliefs already use the correct name. Saves a future migration on a freshly-built table.
- **2026-05-15:** Chose to scope contradiction detection to topical neighborhoods, not globally. Reason: O(N²) over all pairs becomes intractable past ~1k beliefs. Topical scoping via existing KG edges keeps it O(N × avg_neighborhood_size).
- **2026-05-15:** Chose to NOT build a theorem prover. Reason: beliefs are aggregated stances, not formal propositions. A theorem prover would be a much larger architectural commitment for limited additional value at this stage.
- **2026-05-15:** Chose 4-phase rollout instead of one big PR. Reason: each phase is independently useful and testable. B-1 alone (synthesis without contradiction graph) is a measurable improvement. B-2 builds on it. Risk is bounded per phase.
- **2026-05-15:** Chose to keep belief presentation (the `## Active Beliefs` block) at the gateway-execution layer, not in gateway-memory. Reason: matches the audit's M1+M2+M3+M4+M5 direction — prompt formatting belongs at the consumer layer.
- **2026-05-15:** Chose `synthesizer_version` column. Reason: when the BeliefSynthesizer algorithm or prompt changes, we want to re-synthesize old beliefs on demand without losing the historical version. Similar to how migrations have versions.

---

## Open Questions (revisit before Phase B-1 implementation)

1. **Subject canonicalization** — Phase 1 uses exact key match (`user.location`). Should we canonicalize via embedding similarity (`user.location` and `user.address` collapse to one subject) in Phase 1 or defer to Phase 2? Tradeoff: Phase-1 simplicity vs. better belief consolidation.
2. **Confidence formula** — should constituent confidence be a recency-weighted average, or something more sophisticated (e.g., Bayesian update)? Phase 1 should use the simplest version that works; revisit if results are poor.
3. **Belief retrieval in recall** — should beliefs always be surfaced when their constituent facts surface (i.e., dedup facts in favor of their belief)? Or should they be additive? Phase 4 will decide; preview here for visibility.
4. **What's the right default for category weight on beliefs?** I propose 1.7 (above schema at 1.6) since beliefs are more distilled. But this is a guess — should be tuned empirically once Phase B-4 lands.
5. **Should contradiction resolution involve a user prompt?** ("We have conflicting evidence about X — which should we trust?") Probably yes in some cases, but the design here keeps it agent-internal for v1.

---

## What This Doc Is NOT

This is a northstar design, not an implementation plan. Before implementing Phase B-1:

1. Pick the simplest defensible answer to each of the 5 open questions above
2. Sketch the BeliefSynthesizer LLM prompt and test it manually on a real subject from the existing memory_facts table
3. Pilot subject identification on real data — confirm exact-key-match is enough for v1, or escalate to embedding similarity
4. Decide whether to ship Phase B-1 standalone or wait until Phase B-2 design is also signed off

After implementation: validate against a real validation corpus (the runs we haven't completed). Beliefs should make recall demonstrably better for queries that today produce N raw conflicting facts.
