# Implementation review — 2026-07-17

## Result

Clean — ready to commit.

The implementation keeps the public responses and bounded UI fetches unchanged
while moving Engram summary work to sidecar scalar columns.

## Findings reviewed

- **Correctness:** active entity and relationship counts now read the same
  active-row predicates as before without deserializing graph payloads.
- **Compatibility:** normal adapter writes canonicalize relationships before
  persistence; direct counts preserve the established durable-row invariant.
- **Hierarchy:** disabled requests return before accessing `kg_store`; enabled
  requests use indexed layer and inter-cluster fields, and only inspect
  aggregate properties for layer-positive rows.
- **Scale:** the default UI remains capped at 200 entities and 500
  relationships. Live `EXPLAIN QUERY PLAN` against the Engram database showed
  covering indexes for active counts, active layers, and active inter-cluster
  relationships.

## Verification

- `cargo fmt --all -- --check`
- `git diff --check`
- `cargo clippy -p zbot-engram-adapter --tests -- -D warnings`
- `cargo clippy -p gateway --lib -- -D warnings`
- `cargo check -p gateway`
- `cargo test -p zbot-engram-adapter` — 127 tests passed
- `npm --prefix apps/ui test -- --run src/features/observatory/graph-hooks.test.ts src/features/observatory/ObservatoryPage.test.tsx` — 21 tests passed
- Live daemon checks: `/api/graph/stats`, `/api/hierarchy/stats`, and the
  related SQLite query plans succeeded.

## Known unrelated gates

- `cargo test -p gateway --lib` cannot compile an existing test because
  `gateway/src/http/artifacts.rs` calls `expect_err` on an
  `ArtifactResponse` that does not implement `Debug`. This change does not
  touch that type or test.
- Repository-wide spec-status lint reports pre-existing violations in other
  specs. The Observatory spec conforms to the required status and task-list
  metadata.
