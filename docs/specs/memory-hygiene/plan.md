# Plan: Memory Hygiene

- **Spec:** [`spec.md`](spec.md)
- **Status:** Closed

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

This plan is closed as superseded. Do not execute these tasks directly; see the
closure note in [`spec.md`](spec.md). Any remaining hygiene work should be
opened against the active Engram, embedding-backed recall, context capability,
or runtime context-control specs instead of reviving this `knowledge.db`-era
plan.

Implement the hygiene guards at the callers that know intent: recall bounds
retrieval queries before embedding, handoff routes full machine state through
exact-key `ctx` storage, and distillation verifies relationship endpoints before
writing KG edges. Keep the storage layer's existing integrity checks as the
backstop rather than weakening them. Add narrow tests around each failure mode,
then one cross-cutting log/status check so future long sessions expose partial
memory failures as counters instead of unrelated warnings.

## Constraints

- RFC-0002: approve deterministic caller-side memory hygiene guards over prompt
  only, store-only, or new-subsystem approaches.
- RFC-0001 and `runtime-context-control`: durable memory remains in
  `knowledge.db`; live context pruning is out of scope.
- Preserve the existing normal fact quality contract: semantic facts over 800
  characters remain invalid.
- Prefer existing `MemoryFactStore::save_ctx_fact` for full handoff state before
  considering trait or schema changes.

## Construction tests

**Integration tests:**
- `cargo test -p gateway-memory recall`
- `cargo test -p gateway-execution handoff`
- `cargo test -p gateway-execution distillation`
- `cargo test -p zbot-stores-sqlite memory_fact_store knowledge_graph`

**Manual verification:** run or inspect a long-session log after implementation
and verify no recall `input length exceeds context length`, no
`handoff.latest` fact cap rejection, and no distillation FK warning for
relationship writes.

## Tasks

### T1: Recall embedding input is bounded before provider calls

**Depends on:** none

**Touches:** `gateway/gateway-memory/src/recall/mod.rs`,
`gateway/gateway-memory/src/recall/query_gate.rs`,
`gateway/gateway-services/src/embedding_service.rs`

**Mode:** TDD

**Spec mapping:** Acceptance Criteria 1, 2, 8.

**Tests:**
- Add a `gateway-memory` recall test with an embedding client that fails if
  input exceeds the configured cap; assert recall calls it with bounded input.
- Add a recall test where embedding returns an error and lexical/FTS recall
  still returns matching facts.
- Add a recall test where a provider context-length error on the primary
  bounded query retries with a smaller deterministic query before degrading.
- Add a direct-fallback query-gate test proving overlong raw input is capped
  before embedding even when query-gate is disabled or unavailable.

**Approach:**
- Add a small deterministic query hygiene helper near `MemoryRecall::embed_query`
  or query-gate validation.
- Use token-aware counting if an existing encoder is available in this crate;
  otherwise use a conservative UTF-8-safe char cap with tests.
- Preserve the original query for lexical search where useful, but pass only the
  bounded query to the embedding client.
- Add structured fields or a lightweight counter for `recall_embed_input_too_long`
  and `recall_embed_failed`.

**Done when:** oversized recall input cannot reach the embedding client, and
embedding failure does not suppress lexical recall.

### T2: Full handoff payloads persist as exact-key context state

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/sleep/handoff_writer.rs`,
`stores/zbot-stores-sqlite/src/memory_fact_store.rs`,
`stores/zbot-stores-traits/src/memory_facts.rs`

**Mode:** TDD

**Spec mapping:** Acceptance Criteria 3, 4, 5, 8, 9.

**Tests:**
- Add a handoff-writer test with JSON content over 800 characters; assert
  `handoff.latest` and `handoff.<session_id>` persist through `save_ctx_fact` or
  equivalent exact-key storage.
- Keep or add a `zbot-stores-sqlite` test proving oversized normal semantic
  facts still fail validation.
- Add a test proving full handoff JSON is not written as a normal fuzzy
  semantic fact.

**Approach:**
- Change `HandoffWriter::persist` to call `MemoryFactStore::save_ctx_fact` for
  full handoff entries.
- Use stable keys for exact lookup, such as `ctx.<session_id>.handoff.latest`
  and `ctx.<session_id>.handoff.<session_id>`, unless adjacent ctx naming shows
  a better local convention during implementation.
- Optionally write a short semantic summary fact only if it is below the normal
  fact cap; do not truncate full JSON into a semantic fact.
- Add structured fields or counters for `handoff_ctx_saved`,
  `handoff_semantic_fact_saved`, and `handoff_rejected`.

**Done when:** a full handoff larger than 800 characters is persisted without
weakening normal fact validation.

### T3: Distillation never writes FK-invalid relationships

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/distillation.rs`,
`stores/zbot-stores-sqlite/src/kg/storage.rs`

**Mode:** TDD

**Spec mapping:** Acceptance Criteria 6, 7, 8.

**Tests:**
- Add a distillation endpoint-resolution test where stub entity persistence
  fails; assert no relationship write is attempted with the failed stub ID.
- Add a valid relationship test proving existing entity-map, existing graph
  lookup, and successful stub-creation paths still persist relationships.
- Add a dropped-relationship counter/log assertion for unresolved endpoints.

**Approach:**
- Change relationship endpoint resolution to return `Result<Option<EntityId>,
  Error>` or an equivalent explicit success/failure shape instead of always
  returning a string.
- In the relationship loop, write only relationships with two resolved persisted
  endpoint IDs.
- Drop unresolved relationships in the first implementation and count them;
  quarantine can be added later if debugging needs it.
- Keep SQLite FK constraints as the final integrity backstop.

**Done when:** distillation relationship processing produces zero FK warnings
for unresolved endpoint cases and still writes valid relationships.

### T4: Memory hygiene outcomes are observable

**Depends on:** T1, T2, T3

**Touches:** `gateway/gateway-memory/src/recall/mod.rs`,
`gateway/gateway-execution/src/sleep/handoff_writer.rs`,
`gateway/gateway-execution/src/distillation.rs`,
`stores/zbot-stores-sqlite/src/distillation_repository.rs`,
`gateway/src/http/graph.rs`

**Mode:** Goal-based check plus focused tests

**Spec mapping:** Acceptance Criteria 8.

**Tests:**
- Add focused tests for any new stats struct or distillation run fields.
- If only structured logs ship first, add tests around helper functions and run
  `rg -n "recall_embed_input_too_long|handoff_ctx_saved|kg_relationship_dropped" gateway stores`.
- If status endpoints are extended, add/update `gateway` HTTP tests for the new
  fields.

**Approach:**
- Start with structured logs/counters inside the three subsystems.
- Extend persisted distillation stats only for relationship drop counts if the
  existing repository can accept the field without broad migration churn.
- Keep user-facing UI out of scope unless the spec is updated.

**Done when:** each hygiene path emits a machine-searchable reason and count or
structured field.

### T5: Boundary and regression gates pass

**Depends on:** T1-T4

**Touches:** `docs/specs/memory-hygiene/*`

**Mode:** Goal-based check

**Spec mapping:** Acceptance Criteria 1-9.

**Tests:**
- `cargo test -p gateway-memory recall`
- `cargo test -p gateway-execution handoff`
- `cargo test -p gateway-execution distillation`
- `cargo test -p zbot-stores-sqlite memory_fact_store knowledge_graph`
- `cargo check --workspace`
- `rg -n "input length exceeds the context length|handoff.latest: fact content too long|FOREIGN KEY constraint failed" ~/Documents/zbot/logs/zerod.2026-05-31.log` is used only as a baseline comparison, not a passing gate.

**Approach:**
- Run targeted tests after each subsystem lands.
- Run the workspace check after all code changes.
- Audit the diff for no new durable memory store, no `conversations.db`
  handoff writes, and no weakening of the normal fact cap.
- Refresh Codemem with changed files after implementation.

**Done when:** targeted tests and workspace check pass, and the diff stays
inside the RFC/spec boundaries.

## Rollout

Ship as a backend behavior change with no migration required if full handoffs
use existing `ctx` storage. Existing failed handoff attempts are not backfilled.
The behavior is reversible by reverting caller-side routing and guards; no new
durable memory source is introduced.

## Risks

- The recall cap may over-shrink queries and reduce semantic recall relevance;
  lexical fallback should preserve basic recall utility.
- Routing handoffs through `ctx` may require adapting readers that currently
  expect `handoff.latest` as a normal fact key.
- Relationship-drop counters may reveal poor distillation prompt quality, which
  should be handled as a follow-up rather than hidden by unsafe writes.
- Extending persisted distillation stats may require a schema migration if
  structured logs are not enough.

## Changelog

- 2026-05-31: initial plan.
- 2026-05-31: tightened T1 after live logs showed the initial 2,000-character
  cap still exceeded the active Ollama embedding model context; add a smaller
  retry before lexical-only degradation.
