# Spec: SQLite Agent Isolation

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`Entity Dedup as __global__`](../../adr/decisions.md#entity-dedup-as-__global__)
- **Brief:** none
- **Contract:** none
- **Shape:** data

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Restore an executable cross-backend conformance contract for agent-scoped
knowledge-graph listing. A caller requesting one agent's entities may receive
that agent's rows plus intentionally shared `__global__` rows, but must never
receive rows owned by another non-global agent. The SQLite test suite must
prove both halves of that rule without changing the established global entity
deduplication model.

## Boundaries

### Always do

- Treat `__global__` as an explicitly shared scope.
- Apply agent scoping in the storage query before rows are returned.
- Keep a non-ignored regression test for unrelated private-agent exclusion.

### Ask first

- Changing which data is stored under `__global__`.
- Changing the `KnowledgeGraphStore` public method signatures.
- Expanding this work to relationship listing or other graph read paths.

### Never do

- Remove `__global__` visibility from agent-scoped graph reads.
- Treat every globally stored entity as a cross-agent leak.
- Add a new module, dependency, schema migration, or embedding-reindex change.

## Testing Strategy

- **Goal-based contract check:** run the existing cross-backend conformance
  scenario without `#[ignore]`; it must accept requested-agent and
  `__global__` rows while rejecting unrelated private-agent rows.
- **TDD construction regression:** a SQLite unit test seeds requested,
  unrelated, and global rows directly, then proves `list_entities` returns
  exactly the requested plus global set. Direct seeding is required because
  production upserts intentionally normalize new entities to `__global__`.
- **Goal-based integration gates:** formatting, workspace typechecking,
  Clippy, and workspace tests guard against contract or fixture regressions.

## Acceptance Criteria

- [x] Given private rows for `agent-a` and `agent-b`, listing for `agent-a`
      returns only `agent-a` and explicitly shared `__global__` rows; no
      `agent-b` row crosses the storage boundary or is returned.
- [x] An explicitly shared `__global__` entity remains visible to `agent-a`.
- [x] The SQLite `list_entities_respects_agent` conformance test runs by
      default and passes.
- [x] Rust formatting, workspace typechecking, Clippy, and workspace tests pass.

## Assumptions

- Technical: the ignored conformance scenario fails because SQLite upserts both
  test entities into `__global__`, not because the SQL returns an unrelated
  private-agent row (source: targeted failing test plus
  `stores/zbot-stores-sqlite/src/kg/storage.rs`).
- Technical: agent-scoped graph reads intentionally include `__global__`
  entities (source: `docs/adr/decisions.md`, “Entity Dedup as __global__”).
- Technical: the current SQLite query already filters to
  `agent_id = ? OR agent_id = '__global__'` (source:
  `stores/zbot-stores-sqlite/src/kg/storage.rs`).
- Product: P1 is limited to resolving this list-entities isolation debt; the
  embedding-reindex test remains separate (source: user confirmation
  2026-07-30).
- Process: access-control boundary work receives full plan and security review
  (source: `docs/CONVENTIONS.md` and work-loop policy).
