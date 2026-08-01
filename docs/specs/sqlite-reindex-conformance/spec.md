# Spec: SQLite Reindex Conformance

Mode: full (reindex controls destructive vec-table recreation)

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** integration

## Objective

Make SQLite's existing embedding-reindex idempotency contract true and
executable. A reindex request must rebuild vec tables whose live DDL has a
different dimension, while a repeated request for the already-live dimension
must not drop, recreate, or report those tables again.

## Boundaries

### Always do

- Reuse `SqliteKgStore::with_embedding_client`.
- Keep the embedding implementation deterministic and test-only.
- Derive the live dimension from each vec table's SQLite DDL before deciding
  whether that table needs rebuilding.

### Ask first

- Changing the `KnowledgeGraphStore::reindex_embeddings` contract.
- Changing recovery behavior for missing or malformed vec tables.

### Never do

- Add a dependency, embedding backend, schema migration, public-interface
  change, or new module.
- Skip a table when its live dimension cannot be proven to match.

## Testing Strategy

- **TDD:** the existing ignored
  `reindex_idempotent_when_dim_matches` scenario is the red regression. Run it
  without `--ignored`; it must rebuild for dimension 1024 once and report no
  rebuild on the second call.
- **TDD production check:** the existing all-target reindex test must continue
  to prove that mismatched tables rebuild rather than being skipped.
- **Goal-based integration gates:** formatting, workspace typechecking,
  Clippy, and workspace tests guard against fixture regressions.

## Acceptance Criteria

- [x] The SQLite fixture can construct a store with a deterministic embedding
      client of a caller-selected dimension.
- [x] A vec table is skipped only when its live DDL dimension equals the
      requested dimension; missing, malformed, or mismatched tables rebuild.
- [x] Orphan `*__new` tables are cleaned even when every live table already
      matches and no live table is reported as rebuilt.
- [x] `reindex_idempotent_when_dim_matches` runs without `#[ignore]` and passes.
- [x] The first request for dimension 1024 reports the five SQLite reindex
      targets, while the second reports no rebuilt tables.
- [x] Schemas and public traits remain unchanged.
- [x] Rust formatting, workspace typechecking, Clippy, and workspace tests pass.

## Assumptions

- Technical: the ignored scenario fails because `sqlite_store()` uses
  `SqliteKgStore::new` without an embedding client (targeted reproduction,
  2026-07-30).
- Technical: `SqliteKgStore::with_embedding_client` is the established
  production constructor for reindex-capable stores
  (`stores/zbot-stores-sqlite/src/knowledge_graph.rs`).
- Technical: after fixture wiring, the scenario fails because `reindex_all`
  unconditionally calls `reindex_table` for all five targets on every request
  (targeted red test plus `stores/zbot-stores-sqlite/src/reindex.rs`).
- Technical: the current rebuild path also removes orphan `*__new` tables, so
  same-dimension skipping must preserve that cleanup independently
  (`stores/zbot-stores-sqlite/src/reindex.rs`).
- Product: P2 is limited to enabling the existing SQLite reindex conformance
  scenario (user confirmation 2026-07-30).
- Process: discovery escalated this from light to full mode because the root
  fix changes the decision guarding destructive vec-table recreation.
