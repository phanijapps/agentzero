# Plan: SQLite Agent Isolation

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Correct the stale conformance expectation to encode the repository's actual
scope rule: requested-agent plus `__global__`, never an unrelated private
agent. Add a backend-specific SQLite regression that directly seeds all three
scope classes so the SQL boundary is proven independently of the globalizing
upsert path. Remove the stale ignore marker and leave production storage,
schema, and the separate reindex test unchanged.

## Constraints

- Follow the `__global__` entity-dedup decision in
  `docs/adr/decisions.md`.
- Preserve the existing `KnowledgeGraphStore` interface and SQLite schema.
- Keep the change confined to list-entities contract tests and documentation.
- Tempted to introduce a scope-policy abstraction; declining because the
  existing two-scope SQL rule is explicit and already shared by nearby reads.
- Tempted to repair relationship conformance and embedding reindexing;
  declining because both are separate observable contracts.

## Construction tests

**Integration tests:** the enabled SQLite conformance scenario and the full
workspace test suite.

**Manual verification:** inspect `GraphStorage::list_entities` to confirm the
agent/global predicate is applied in SQL before row mapping, and inspect the
diff to confirm no production SQL, schema, or public interface changed.

## Design (LLD)

### Design decisions

- Define authorized output as `{requested agent, __global__}` rather than exact
  equality with the requested agent. Traces to: AC1, AC2.
- Seed private rows directly in the SQLite storage test so it can distinguish a
  true other-agent leak from intentional global visibility. Traces to: AC1.

### Data & schema

- No schema change. The test exercises existing `kg_entities.agent_id` values
  for `agent-a`, `agent-b`, and `__global__`. Traces to: AC1, AC2.

### Failure, edge cases & resilience

- A filter that accidentally broadens to all agents fails the direct storage
  regression.
- A filter that accidentally removes `__global__` fails both the direct
  storage regression and the corrected conformance contract.
- A backend that preserves agent-specific upserts remains valid under the
  cross-backend contract.

## Tasks

### T1: Agent-scoped listing contract is executable and precise

**Depends on:** none

**Touches:** `stores/zbot-stores-conformance/src/lib.rs`,
`stores/zbot-stores-sqlite/src/kg/storage.rs`,
`stores/zbot-stores-sqlite/tests/conformance.rs`,
`docs/specs/sqlite-agent-isolation/**`, `docs/specs/README.md`

**Tests:**

- TDD: add
  `kg::storage::tests::list_entities_scopes_to_requested_agent_and_global`,
  directly seed `agent-a`, `agent-b`, and `__global__` rows, and verify listing
  `agent-a` returns only `agent-a` and `__global__` (AC1, AC2).
  stub: true
- Goal-based: run `cargo test -p zbot-stores-sqlite --test conformance
  list_entities_respects_agent -- --exact` without `--ignored` (AC3).
- Goal-based: run Rust format, workspace check, Clippy, and workspace tests
  (AC4).

**Approach:**

- Correct the generic conformance assertion to allow only requested-agent or
  `__global__` rows.
- Add the direct SQLite storage regression using existing test helpers.
- Remove only the stale isolation ignore marker and comment.

**Done when:** both targeted tests and all workspace gates pass, with no
production SQL or schema diff.

## Rollout

Test-only contract correction; no runtime rollout or migration is required.

## Risks

- Weakening the generic assertion too far could hide a real private-agent leak;
  the SQLite direct-seeding test prevents that.
- Removing global visibility would silently break the intentional cross-agent
  graph dedup model; AC2 prevents that.

## Changelog

- 2026-07-30: initial plan after Engram and filesystem tracing showed the
  ignored test conflicted with the established `__global__` contract.
