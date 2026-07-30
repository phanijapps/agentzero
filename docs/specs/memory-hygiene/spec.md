# Spec: Memory Hygiene

- **Status:** Archived
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0002: Memory Hygiene; RFC-0001: Unified Compaction and Memory Policy; [`runtime-context-control`](../runtime-context-control/spec.md)

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make zbot's durable-memory pipeline resilient to malformed or oversized memory
inputs during long goal-oriented sessions. Recall must degrade to lexical search
instead of losing memory when an embedding query is too large; handoff continuity
must persist as exact-key context state instead of being rejected by the normal
fact cap; and distillation must never attempt a knowledge-graph relationship
write unless both endpoints resolve to persisted entities. The user-visible
result is fewer silent memory losses: long sessions should end with usable
handoffs, recall should still return results when embeddings fail, and KG writes
should report partial success without SQLite foreign-key warnings.

## Closure

Closed as superseded. The original problem set was split and resolved by later
work:

- Recall query embedding safety and fail-closed behavior now live under
  [`embedding-backed-memory-recall`](../embedding-backed-memory-recall/spec.md).
- Durable memory/knowledge ownership moved to Engram under
  [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md).
- Context packet observability, recall atoms, and tool/context boundaries live
  under
  [`context-capability-registry`](../context-capability-registry/spec.md).
- Live compaction remains covered by
  [`runtime-context-control`](../runtime-context-control/spec.md).

Do not implement this draft as written; its `knowledge.db` and broad lexical
fallback assumptions are stale after the Engram and embedding-backed recall
cutovers.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep `knowledge.db` as the durable memory source of truth for facts, context
  state, KG entities, relationships, episodes, wiki, and procedures.
- Preserve the normal 800-character semantic fact cap for user/domain/
  correction/strategy facts.
- Keep full handoff payloads out of fuzzy semantic recall unless a separate
  short semantic summary is intentionally written.
- Treat embedding failure as degraded recall, not a hard recall failure.
- Validate KG relationship endpoints before relationship writes and record
  dropped relationship counts when endpoints cannot be persisted.

### Ask first

- Changing the `MemoryFactStore` trait shape or the `memory_facts` schema beyond
  using existing `save_ctx_fact` / exact-key context storage.
- Moving handoff state into `conversations.db` instead of `knowledge.db`.
- Adding a new background queue, new database, or external graph backend.
- Changing distillation prompt semantics beyond what is required to support the
  deterministic hygiene guards.
- Exposing new user-facing UI surfaces for memory health beyond logs or existing
  API/status endpoints.

### Never do

- Never weaken or remove the normal semantic fact length validation to make
  handoff writes pass.
- Never replace `knowledge.db` durable memory with markdown files, `memory.json`,
  or `conversations.db`.
- Never write a KG relationship with guessed endpoint IDs after entity
  persistence failed.
- Never make LLM prompt compliance the only protection against oversized recall
  queries, oversized handoffs, or invalid relationships.
- Never change runtime live-context pruning behavior as part of this feature.

## Testing Strategy

- Recall embedding guard: **TDD**. The behavior is a compact invariant: long
  recall queries are capped or transformed before embedding, and recall still
  returns lexical results when embedding fails.
- Handoff persistence: **TDD**. Full handoff JSON should persist through the
  exact-key context path even when it exceeds the normal fact cap; normal facts
  must still reject oversized semantic content.
- KG relationship integrity: **TDD**. Distillation must not call relationship
  storage when endpoint creation or lookup fails, and valid relationships still
  persist.
- Observability: **goal-based check plus focused tests**. Structured counters or
  log fields should exist for each hygiene outcome, and status plumbing should
  compile without broad UI changes.
- Regression gates: **goal-based check**. Targeted `cargo test` commands for
  `gateway-memory`, `gateway-execution`, and `zbot-stores-sqlite` must pass.

## Acceptance Criteria

- [ ] Recall input passed to the embedding client is deterministically bounded
  before embedding; an oversized raw query cannot produce provider
  `input length exceeds context length` errors from the recall path.
- [ ] If recall embedding fails, `MemoryRecall::recall_unified` still runs
  lexical/FTS recall and returns any matching non-vector results instead of
  returning an empty result solely because embedding failed.
- [ ] Full `handoff.latest` and `handoff.<session_id>` payloads are persisted
  through exact-key `ctx` storage or an equivalent machine-state path exempt
  from normal semantic fact validation.
- [ ] Normal semantic fact categories still reject content over the existing
  800-character cap.
- [ ] Handoff persistence may optionally write a short semantic summary, but
  the full handoff JSON is not inserted into fuzzy semantic recall.
- [ ] Distillation only writes KG relationships when both endpoints resolve to
  persisted entity IDs.
- [ ] If a relationship endpoint cannot be persisted or resolved, the
  relationship is dropped or quarantined and counted; no SQLite foreign-key
  warning is emitted for that relationship.
- [ ] Memory hygiene outcomes are observable with explicit counters or
  structured log fields for recall embedding failures, handoff routing, and KG
  relationship drops.
- [ ] The implementation adds no replacement durable memory store and does not
  route durable memory writes away from `MemoryFactStore` / `knowledge.db`.

## Assumptions

- Technical: `MemoryFactStore` already exposes `save_ctx_fact`, and the SQLite
  implementation stores `ctx` facts as exact-key machine state without
  embedding generation (source:
  `stores/zbot-stores-traits/src/memory_facts.rs`;
  `stores/zbot-stores-sqlite/src/memory_fact_store.rs`).
- Technical: `MemoryRecall::embed_query` currently sends the provided text
  directly to the embedding client and logs provider failures before returning
  `None` (source: `gateway/gateway-memory/src/recall/mod.rs`).
- Technical: the query gate already truncates LLM-reformulated subqueries by
  `max_subquery_len`, but direct fallback can still return raw input (source:
  `gateway/gateway-memory/src/recall/query_gate.rs`).
- Technical: `HandoffWriter::persist` currently writes handoff JSON through
  `save_fact`, so full handoffs are subject to normal fact validation (source:
  `gateway/gateway-execution/src/sleep/handoff_writer.rs`).
- Technical: `SessionDistiller::resolve_relationship_endpoint` can return a
  stub ID after stub persistence fails, allowing a later FK-invalid
  relationship write attempt (source:
  `gateway/gateway-execution/src/distillation.rs`).
- Process: this feature follows RFC-0002 and the local spec shape used by
  `docs/specs/runtime-context-control/`; no `docs/CONVENTIONS.md` or
  `docs/CHARTER.md` exists in this workspace (source: repository read
  2026-05-31).
- Product: the user confirmed these hygiene failures happen often and requested
  spec and planning (source: user confirmation 2026-05-31).
