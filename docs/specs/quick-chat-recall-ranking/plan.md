# Plan: Quick Chat recall ranking

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog at the bottom.

## Approach

Repair the ranking contract at its two scale boundaries, then reserve a
query-scoped profile slot before generic RRF/MMR candidates are selected. The
profile lookup uses existing authorized fact natural keys and stays inside
`MemoryRecall`; it introduces neither settings state nor a new service.

## Constraints

- No storage schema, API, dependency, UI, or prompt-template change.
- Preserve all `UnifiedRecallScope` visibility checks.
- Do not broaden the existing query-gate feature in this fix; canonical profile
  selection addresses the observed Quick Chat miss without relying on an LLM
  rewrite.

## Construction tests

**Integration tests:** existing `MemoryRecall` test harness proves that profile
facts are included or omitted by observable query intent.

**Post-deployment verification:** query the local memory endpoint with a
location-dependent prompt and confirm a canonical profile key is returned
without exposing profile content in logs.

## Design (LLD)

### Design decisions

- Apply a monotonic bounded transform to each fused RRF score; this retains
  order and a `[0, 1)` score contract without saturation.
- Normalize relevance inside MMR by the candidate-set maximum, leaving returned
  RRF scores unchanged while making λ meaningful against cosine similarity.
- Fetch at most one exact canonical profile fact per needed slot, prepend it to
  the final result list, and deduct that count from generic recall budget.

### Behavior & rules

- Identity words select `user.name` then `user.identity`.
- Local-context words select canonical location keys, never an `unknown` key.
- Selection considers current agent-scoped facts before global facts visible to
  the authorized agent; generic recall fills the remaining budget.

### Failure, edge cases & resilience

- An absent or unsupported profile lookup is a no-op; normal unified recall
  continues.
- A profile slot never bypasses the caller's result limit or authorization.

## Tasks

### T1: Preserve Engram hybrid score ordering — Done

**Depends on:** none

**Touches:** `stores/zbot-engram-adapter/src/stores/memory_facts.rs`

**Tests:**

- TDD: two distinct fused scores remain distinct and ordered after hybrid-score
  normalization (AC 1).
  stub: false

**Approach:**

- Add a red unit test around the hybrid score normalizer.
- Apply a monotonic bounded transform to each fused score rather than
  saturating it independently.

**Done when:** the Engram adapter test proves ranking is monotonic and the
focused crate test is green.

### T2: Make MMR relevance scale-invariant — Done

**Depends on:** T1

**Touches:** `gateway/gateway-memory/src/recall/mmr.rs`

**Tests:**

- TDD: a low-magnitude RRF candidate set selects the same items as a positive
  scalar multiple at the same λ (AC 2).
  stub: false

**Approach:**

- Normalize candidate relevance by the maximum positive score inside MMR.
- Preserve the original candidate score and output ordering contract.

**Done when:** the MMR regression test is green and existing MMR tests pass.

### T3: Reserve query-scoped canonical profile facts — Done

**Depends on:** T1, T2

**Touches:** `gateway/gateway-memory/src/recall/mod.rs`

**Tests:**

- TDD: a location-dependent query includes one canonical current location fact
  before generic recall, while an unrelated query includes none (AC 3, AC 4).
- TDD: `*.unknown` profile facts are excluded; the lookup also defensively
  skips superseded facts (AC 3).
  stub: false

**Approach:**

- Add small private query-intent and canonical-key helpers in the existing
  recall module.
- Fetch only authorized global profile facts through `MemoryFactStore`, reserve
  their budget, and prepend them to generic unified recall output.

**Done when:** focused unified-recall tests prove the observable profile
selection behavior and all recall tests pass.

## Rollout

Normal daemon deployment; no migration or configuration change. Rollback is a
code revert because no stored data is modified.

## Risks

- Key naming may vary across historical profile facts; the selection helper
  must use a small explicit canonical allowlist and safely no-op on misses.
- Profile facts consume part of Quick Chat's five-item budget only when the
  request explicitly needs identity or local context.

## Changelog

- 2026-07-17: Initial full-mode bug-fix plan from the retrieval investigation.
- 2026-07-17: Implemented monotonic hybrid-score normalization, scale-invariant
  MMR, and authorized query-scoped profile selection from durable memory.
- 2026-07-17: Added the deployed `user.location.home_base` natural key and
  agent-scoped profile lookup after live API validation exposed the legacy
  global-only assumption.
