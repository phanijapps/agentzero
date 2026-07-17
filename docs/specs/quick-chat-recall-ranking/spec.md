# Spec: Quick Chat recall ranking

- **Status:** Shipped
- **Owner:** maintainer
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Quick Chat must use durable memory facts to supply identity or local context
when a request needs it, without copying profile data into settings. Hybrid
retrieval must preserve relative relevance through source fusion and MMR so
that relevant facts are selected before the bounded context packet is built.

## Boundaries

### Always do

- Read profile values from the authorized, canonical memory-fact store.
- Preserve existing authorization, visibility, and prompt-budget rules.
- Cover each changed ranking or selection invariant with a regression test.

### Ask first

- Expanding profile selection beyond identity and location facts.
- Changing the public memory-search API or stored fact schema.

### Never do

- Copy profile data into settings or another static configuration file.
- Inject profile facts for unrelated requests, or inject a low-confidence
  `*.unknown` profile fact as a canonical value.
- Add a dependency, new top-level module boundary, UI change, or data migration.

## Testing Strategy

- Hybrid score normalization and MMR score-scale invariance: TDD unit tests,
  because both are pure ranking invariants.
- Profile-context selection: TDD integration-style recall test using the
  existing in-memory/SQLite-backed memory-store test harness, because it
  verifies the observable recalled items rather than a mocked call shape.
- Crate checks: goal-based `cargo test`, `cargo clippy`, and `cargo fmt`.

## Acceptance Criteria

- [x] Hybrid fact search preserves a strict ordering for distinct fused scores;
  its top results do not all collapse to `1.0`.
- [x] MMR produces the same selection when all candidate relevance scores are
  multiplied by a positive constant, while retaining the configured relevance
  versus diversity trade-off.
- [x] A location-dependent Quick Chat request includes one current canonical
  location fact from memory before generic recalled results, without reading
  settings or selecting an `unknown` location fact.
- [x] An identity-dependent request includes one current canonical name or
  identity fact from memory; unrelated requests do not gain a profile fact.
- [x] Existing context-packet authorization and token-budget behavior remains
  unchanged for the final selected list.

## Assumptions

- Technical: Quick Chat automatic recall reaches `recall_unified_outcome_scoped`
  and facts are available through `MemoryFactStore` (source:
  `gateway/gateway-execution/src/invoke/unified_recall_adapter.rs`).
- Technical: profile fact natural-key lookup is supported by the existing
  `MemoryFactStore` trait and Engram adapter (source:
  `stores/zbot-stores-traits/src/memory_facts.rs`).
- Product: profile data is dynamic stored memory, not settings (source: user
  confirmation 2026-07-17).
- Process: this is a full-mode fix because it changes the LLM/agent context
  boundary (source: `work-loop` risk trigger).
