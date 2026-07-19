# Bi-Temporal Memory — Wiring & Point-in-Time Recall

**Date:** 2026-05-15
**Status:** Northstar — design ready, implementation queued behind Phase 4 (Belief Network) prioritization
**Context:** Borrowed from Yang et al. (arxiv 2602.05665) and Du (arxiv 2603.07670) — both surveys flag bi-temporal modeling (Graphiti pattern) as the missing piece in production agent-memory systems. AgentZero already has the columns; this doc wires them up.
**Related:**
- `[[project_reflective_memory_roadmap]]` — Phase 4 Belief Network is what this unblocks
- `[[project_memory_crate_extraction]]` — wiring touches gateway-memory + zbot-stores-sqlite
- PR #139 (merged to develop via #141) — recall double-boost fix
- PR #142 — recall supersession-semantic bug fix (prerequisite for this work)

---

## What This Enables

Two capabilities z-Bot doesn't have today:

1. **Point-in-time recall** — "What did the agent believe about X at time T?" Today recall returns the current value only. With this, the agent can answer historical queries: "User worked at Anthropic February-April 2026" remains retrievable for context about that period even after newer facts overwrite the current employer.

2. **Phase 4 Belief Network** — the last unbuilt phase of the reflective memory roadmap. A "belief" becomes a slice through time (valid for an interval) rather than a single mutable row. ConflictResolver gains the ability to record *when the world changed*, not just *which fact won*. Future contradictions can be reasoned about temporally: "this contradicts the belief held 2026-02 through 2026-04" is more useful than "this contradicts the older fact."

---

## The Surprise: ~70% is already done

Auditing the codebase before designing turned up that bi-temporal infrastructure is far more complete than expected:

### Schema (done)

`memory_facts` (`knowledge_schema.rs:223-247`) has:
- `valid_from TEXT` — when fact became true (column exists, currently always NULL)
- `valid_until TEXT` — when fact stopped being true (column exists, currently NULL except after supersession)
- `superseded_by TEXT` — ID of newer fact that replaces this (column exists, populated by ConflictResolver)
- `created_at TEXT` — when the row was inserted (always populated)
- `updated_at TEXT` — when the row was last modified

`kg_entities` (`knowledge_schema.rs:82-103`) has the same bi-temporal columns: `valid_from`, `valid_until`, `invalidated_by`, plus `first_seen_at`, `last_seen_at`, `last_accessed_at`.

`kg_relationships` (`knowledge_schema.rs:113-135`) is the **odd one out**: it has `valid_at` (singular) and `invalidated_at` instead of the symmetric `valid_from`/`valid_until` pair. Asymmetric but workable.

### Supersession writer (done correctly)

`memory_repository.rs:465-473`:
```rust
pub fn supersede_fact(&self, old_id: &str, new_id: &str) -> Result<(), String> {
    self.db.with_connection(|conn| {
        conn.execute(
            "UPDATE memory_facts SET valid_until = ?3, superseded_by = ?1 WHERE id = ?2",
            params![new_id, old_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
}
```

This is already bi-temporally correct. When a fact is superseded, the writer sets BOTH `valid_until = now` (the moment the world changed) AND `superseded_by = winner_id`. The loser's truth-interval ends; the winner's begins.

### Recall semantic distinction (done — PR #142)

PR #142 fixed `apply_class_aware_penalty` to check `superseded_by.is_some()` instead of `valid_until.is_some()`. Without that fix, any bi-temporal `valid_until` write would silently penalize valid historical facts. PR #142 is a hard prerequisite for everything in this doc.

### INSERT statement (column-aware)

`memory_repository.rs:168` already includes `valid_from`, `valid_until`, `superseded_by` in the INSERT column list and parameter list. The writer accepts these values — every CALLER just passes `None`.

---

## What's Missing

Four concrete gaps:

### Gap 1 — `valid_from` is never populated on creation

Every caller of `save_fact` / the underlying INSERT passes `valid_from: None`. Result: facts created today have no recorded "began being true at" time. Without `valid_from`, point-in-time queries can't tell whether a fact was valid at the query time.

**Fix:** The `save_fact` trait method (`zbot-stores-traits/src/memory_facts.rs`) and its impls need a `valid_from: Option<DateTime>` parameter (default `Some(Utc::now())`). Callers explicitly opt out by passing `None` only for facts that have always been true (categories like `procedural`, `convention`). All other categories default to `valid_from = created_at`.

Caller sites to update (from grep `save_fact` excluding tests):
- `runtime/agent-tools/src/tools/memory.rs:541` — `action_save_fact` (the agent-callable tool path)
- `gateway/gateway-memory/src/sleep/corrections_abstractor.rs:135` — abstractor-produced schema facts
- Plus any KG-side save paths in `synthesizer.rs`, `decay.rs`, `orphan_archiver.rs`, `kg_backfill.rs` if they write through the same trait

### Gap 2 — No point-in-time recall API

`recall_facts_prioritized` (`stores/zbot-stores-traits/src/memory_facts.rs:112`) takes `(agent_id, query, limit)`. There's no way to ask "what was true at time T?" — the SQL filter implicitly assumes "now."

**Fix:** Add an optional `as_of: Option<DateTime>` parameter. When `None` (default), behavior unchanged: returns currently-valid facts. When `Some(T)`, filter clause becomes:

```sql
AND (valid_from IS NULL OR valid_from <= ?as_of)
AND (valid_until IS NULL OR valid_until > ?as_of)
```

Same filter applied to `search_memory_facts_hybrid` (`memory_facts.rs:351`) for symmetry across the entire retrieval surface.

This also unlocks a subtle correctness fix for default recall: today, a fact with `valid_until` set in the past (a genuine time-bounded fact whose interval ended) would still be retrievable. After this change, such facts are correctly excluded from "now" queries.

### Gap 3 — kg_relationships schema asymmetry

`kg_relationships` uses `valid_at` (single timestamp) + `invalidated_at`, while `kg_entities` and `memory_facts` use `valid_from` + `valid_until`. The semantic is the same but the column names differ.

**Options:**

- **(a)** Migration: rename `valid_at` → `valid_from`, add `valid_until`, deprecate `invalidated_at` (~½ day). Cleaner long-term.
- **(b)** Accept the asymmetry and document it (~0 days). Cheaper but adds cognitive load forever.

Recommendation: **(a)**. The migration is straightforward (SQLite ALTER TABLE + a single UPDATE statement to copy `invalidated_at` into the new `valid_until` column). One-time cost, permanent clarity.

### Gap 4 — ConflictResolver doesn't write `valid_from` on the winner

Today ConflictResolver calls `supersede_fact(loser_id, winner_id)` which sets the loser's `valid_until = now` and `superseded_by = winner_id`. But it doesn't touch the winner. The winner's `valid_from` stays NULL (because Gap 1 — no writer populates it on creation).

After Gap 1 is fixed, new facts created today will have `valid_from = created_at` populated. So this gap mostly resolves itself once Gap 1 lands. The remaining sub-task: ensure that when a winner is created/updated through ConflictResolver's path, its `valid_from` reflects the transition moment (which may be different from its `created_at` if the fact existed before the conflict).

**Fix:** In `conflict_resolver.rs` resolution path, when promoting a winner whose creation predates the conflict, explicitly UPDATE its `valid_from = max(winner.valid_from, loser.valid_until)`. Edge-case only; default path needs no change.

---

## Design

### Sequence

The four gaps have a natural order:

1. **Gap 1 first** (~1 day) — wire `valid_from` on the create path. Unblocks everything else. Lowest-risk change.
2. **Gap 2 second** (~1-2 days) — add the `as_of` parameter to recall. Backwards-compatible since default is `None`. After this lands, the agent can do point-in-time queries.
3. **Gap 3 third** (~½ day) — kg_relationships migration. Independent of Gaps 1+2; can be done in parallel.
4. **Gap 4 last** (~½ day) — ConflictResolver winner-side UPDATE for the cross-conflict-boundary edge case.

Total: **~3-4 days**, less than the original 5-day estimate because the supersession writer and recall semantic-bug fix are already done.

### File changes

**Phase 1 (Gap 1) — populate valid_from on creation:**
- `stores/zbot-stores-traits/src/memory_facts.rs` — extend `save_fact` trait signature to take `valid_from: Option<DateTime<Utc>>`
- `stores/zbot-stores-sqlite/src/memory_repository.rs:168` — pass `valid_from` through to INSERT
- `stores/zbot-stores-sqlite/src/memory_fact_store.rs` — adapter passes through
- `runtime/agent-tools/src/tools/memory.rs:541` — `action_save_fact` defaults `Utc::now()`
- `gateway/gateway-memory/src/sleep/corrections_abstractor.rs:135` — abstractor defaults `Utc::now()`
- Backfill migration (one-time): `UPDATE memory_facts SET valid_from = created_at WHERE valid_from IS NULL`

**Phase 2 (Gap 2) — point-in-time recall:**
- `stores/zbot-stores-traits/src/memory_facts.rs:112` — add `as_of: Option<DateTime<Utc>>` to `recall_facts_prioritized`
- `stores/zbot-stores-traits/src/memory_facts.rs:351` — same for `search_memory_facts_hybrid`
- `stores/zbot-stores-sqlite/src/memory_repository.rs` — SQL filter clause as documented
- `gateway/gateway-memory/src/recall/mod.rs` — pass `as_of` through from callers; default `None`
- `runtime/agent-tools/src/tools/memory.rs` — agent-callable tool gains optional `as_of` arg in the JSON schema; default omitted = current

**Phase 3 (Gap 3) — kg_relationships symmetry:**
- New migration: `stores/zbot-stores-sqlite/migrations/v25_kg_relationships_bitemporal.sql`
  - `ALTER TABLE kg_relationships ADD COLUMN valid_from TEXT`
  - `UPDATE kg_relationships SET valid_from = valid_at WHERE valid_from IS NULL`
  - `ALTER TABLE kg_relationships ADD COLUMN valid_until TEXT`
  - `UPDATE kg_relationships SET valid_until = invalidated_at WHERE valid_until IS NULL`
  - Keep `valid_at` and `invalidated_at` as legacy (don't drop in this migration; deprecate over time)
- `stores/zbot-stores-sqlite/src/knowledge_schema.rs:113-135` — update CREATE TABLE for fresh DBs
- All writers in `synthesizer.rs`, `decay.rs`, `orphan_archiver.rs`, `kg_backfill.rs` — write to new columns

**Phase 4 (Gap 4) — winner `valid_from` on cross-boundary supersession:**
- `gateway/gateway-memory/src/sleep/conflict_resolver.rs` — in the resolution path, after `supersede_fact(loser, winner)`, conditionally UPDATE the winner's `valid_from`
- `stores/zbot-stores-sqlite/src/memory_repository.rs` — add `promote_winner_valid_from(winner_id, transition_time)` helper

### Migration shape

Two migrations total:

```
migrations/v25_memory_facts_valid_from_backfill.sql
  UPDATE memory_facts SET valid_from = created_at WHERE valid_from IS NULL;

migrations/v26_kg_relationships_bitemporal.sql
  ALTER TABLE kg_relationships ADD COLUMN valid_from TEXT;
  ALTER TABLE kg_relationships ADD COLUMN valid_until TEXT;
  UPDATE kg_relationships SET valid_from = valid_at WHERE valid_from IS NULL;
  UPDATE kg_relationships SET valid_until = invalidated_at WHERE valid_until IS NULL;
```

Both are idempotent and additive. Existing data preserved. No destructive changes.

---

## Test Plan

### Unit (Phase 1)
- `memory_repository.rs::test_save_fact_populates_valid_from` — insert a fact, assert `valid_from = created_at`
- `memory_repository.rs::test_save_fact_respects_explicit_valid_from` — caller passes `Some(specific_time)`, assert stored

### Unit (Phase 2)
- `memory_repository.rs::test_recall_as_of_excludes_future_facts` — fact with `valid_from = 2026-06-01`, query `as_of = 2026-05-15` → not returned
- `memory_repository.rs::test_recall_as_of_excludes_past_facts` — fact with `valid_until = 2026-04-01`, query `as_of = 2026-05-15` → not returned
- `memory_repository.rs::test_recall_as_of_includes_active_facts` — fact valid 2026-03 to 2026-06, query `as_of = 2026-05-15` → returned
- `memory_repository.rs::test_recall_default_excludes_time_bounded_past` — fact with `valid_until = 2026-04-01`, default query (as_of = None means "now" = 2026-05-15) → NOT returned. Locks in the correctness fix.

### Unit (Phase 3)
- `memory_repository.rs::test_kg_relationship_writes_use_new_columns` — INSERT then SELECT, assert `valid_from`/`valid_until` are populated (not just `valid_at`/`invalidated_at`)

### Integration (Phase 4)
- `gateway-memory/tests/conflict_resolver_bitemporal.rs::test_supersession_records_transition_interval` — write fact A at t0, supersede with fact B at t1 → assert A has `valid_from = t0, valid_until = t1`, B has `valid_from = t1, valid_until = NULL`
- `gateway-memory/tests/recall_bitemporal.rs::test_point_in_time_returns_pre_supersession_value` — after the above setup, query `as_of = t0.5` → returns A, not B

### Regression
- All existing `apply_class_aware_penalty` tests must still pass (they cover the recall semantic-distinction already fixed by PR #142)
- All existing recall tests must still pass when `as_of = None`

---

## Out of Scope (v1)

- **Hypergraph edges** (Yang survey) — separate concern, not coupled to bi-temporal
- **`recorded_at` distinction** (when we learned vs when it became true) — defer; `created_at` is sufficient for v1 since z-Bot doesn't record historical facts. Revisit if/when we add a "backdate this fact" capability.
- **UI surfacing of temporal intervals** — defer to a follow-up UI PR
- **Cross-ward temporal queries** — ward scoping orthogonal to time; existing filter behavior unchanged
- **Temporal queries through the agent-callable memory tool** — Phase 2 adds the API surface but the tool's JSON schema for `as_of` is optional; agents continue to default to current queries unless explicitly asking for history
- **Belief Network actual implementation** — this doc unblocks it, doesn't build it. The Belief Network needs its own design (multi-fact temporal reasoning, contradiction graphs, confidence propagation).

---

## Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Migration backfills `valid_from = created_at` for facts that were originally backdated (e.g., user manually entered historical data) | Low | We don't currently support backdating; all existing facts were created at their `created_at` time by definition |
| `as_of` parameter creates a new code path that diverges from default recall | Medium | Single SQL filter clause; same path, just additional WHERE conditions when `as_of.is_some()` |
| ConflictResolver's existing supersession logic interacts poorly with newly-populated `valid_from` on losers | Low | The supersession writer already sets `valid_until = now` on losers; populating their `valid_from` on creation just completes the interval. No conflict. |
| kg_relationships migration leaves dual columns (`valid_at` + `valid_from`) for transition period | Low | Document the deprecation; v26+ writers use new columns; consider dropping old columns in a later migration once all code paths are updated |
| Tests pass but real-world Phase 4 work surfaces edge cases | Medium | Phase 4 (Belief Network) is a separate doc; surface there and iterate |

---

## Decision Log

- **2026-05-15:** Audit revealed ~70% of bi-temporal infrastructure already exists. Schema columns present on `memory_facts` and `kg_entities`. Supersession writer correct. Only `valid_from` population, point-in-time recall API, kg_relationships symmetry, and one ConflictResolver edge case remain. Re-scoped from "5 days, schema migration + wiring" to "3-4 days, wiring only."
- **2026-05-15:** Chose to recommend kg_relationships schema migration (option a) over accepting asymmetry (option b). The cognitive cost of "memory_facts uses valid_from but kg_relationships uses valid_at" forever outweighs the ½-day migration cost.
- **2026-05-15:** Deferred `recorded_at` column. z-Bot doesn't backdate facts today. Adding the column without a use case is speculative; the survey papers mention it but don't make a strong case for current systems.
- **2026-05-15:** PR #142 (`fix/recall-supersession-conflation`) is a hard prerequisite. Without it, any bi-temporal `valid_until` write silently penalizes the fact in recall. Don't start this work until #142 merges to develop.

---

## What This Doc Is NOT

This is a wiring design. It does NOT specify:

- The Belief Network query language (Phase 4 — separate doc)
- Multi-fact temporal contradiction reasoning (Phase 4)
- UI for showing fact history to humans
- Cross-daemon temporal sync (Pattern 4 — separate doc)

Implementation should land in 4 phases (1 → 2 → 3 → 4) as separate PRs, each independently testable. Don't bundle them — they have different blast radii and rollback profiles.

Before implementing, confirm:
1. PR #142 is merged to develop
2. Phase 4 (Belief Network) is actually the next priority (vs. Self-RAG retrieval gate, the other top recommendation from the 2026-05-15 survey synthesis)
3. The kg_relationships migration plays nicely with any in-flight schema work
