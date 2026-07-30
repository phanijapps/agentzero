# Plan: SQLite Reindex Conformance

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Add a dimension-configurable deterministic embedding client to the existing
SQLite integration-test fixtures. Reuse the vec-table DDL dimension reader so
`reindex_all` filters out already-matching targets while rebuilding targets
that are missing, malformed, or mismatched. Then remove the scenario's stale
ignore marker.

Tempted to give every SQLite test fixture an embedding client; declining
because it would change unrelated test behavior. Tempted to generalize a
shared mock-embedding crate; declining because one local fixture is sufficient.
Tempted to persist a separate dimension marker; declining because live
`sqlite_master` DDL is already the vector index's source of truth.

## Design (LLD)

- **Components:** `SqliteKgStore::reindex_embeddings` continues delegating to
  `reindex::reindex_all`; `reindex_all` owns the per-target skip/rebuild
  decision.
- **Data:** each target's `FLOAT[N]` width is read from `sqlite_master`.
- **Failure behavior:** an unreadable dimension is treated as not matching so
  the existing rebuild path can repair the table.

## Tasks

### T1: Enable SQLite reindex conformance

**Depends on:** none

**Touches:** `stores/zbot-stores-sqlite/tests/conformance.rs`,
`stores/zbot-stores-sqlite/src/reindex.rs`,
`stores/zbot-stores-sqlite/src/vector_index.rs`,
`docs/specs/sqlite-reindex-conformance/**`, `docs/specs/README.md`

**Mode:** TDD

**Tests:**

- Existing red regression:
  `cargo test -p zbot-stores-sqlite --test conformance
  reindex_idempotent_when_dim_matches -- --exact`.
  stub: true
- Add red `reindex` unit tests proving a missing target and a malformed target
  rebuild while matching siblings are skipped (AC2).
  stub: true
- Add a red `reindex` unit test proving a matching live table plus an orphan
  `*__new` table performs cleanup without reporting a live rebuild (AC3).
  stub: true
- Green verification runs the same scenario without `--ignored`, followed by
  the Rust workspace gates.

**Approach:**

- Implement a conformance-local `EmbeddingClient` with a stored dimension.
- Add a reindex-capable local fixture constructor using
  `SqliteKgStore::with_embedding_client`.
- Make the existing DDL dimension reader crate-visible and use it to skip only
  targets that already match the requested dimension.
- Keep the all-target unit test mismatched so it continues proving rebuild
  coverage.
- Clean all orphan `*__new` tables before evaluating live dimensions.
- Remove only the stale ignore comment and attribute.
