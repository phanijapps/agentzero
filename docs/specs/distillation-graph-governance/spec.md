# Spec: Distillation Graph Governance

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0011
- **Contract:** none
- **Shape:** data

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Session distillation preserves durable facts through the existing memory-store
contract while preventing unclassified entities, free-form relationship types,
and relationship-only stub entities from becoming graph topology. The change
uses only the existing `KnowledgeGraphStore` and Engram adapter path; it does
not alter historical graph records.

## Boundaries

### Always do

- Route graph writes through `KnowledgeGraphStore`.
- Retain distilled facts through the existing memory-store path.
- Emit structured diagnostics for graph candidates omitted by governance.
- Strip adapter control metadata from LLM-extracted, structured-ingest, and
  Ward-artifact graph properties; scope and governance selection come only
  from trusted runtime/configuration context.

### Ask first

- Rebuild, merge, archive, or delete existing graph records.
- Broaden the built-in entity or relationship vocabulary.

### Never do

- Write to an Engram SQLite database directly.
- Create graph entities solely to satisfy a relationship endpoint.
- Add a new persistence backend, external dependency, or top-level module.

## Testing Strategy

Governance and normalization rules use TDD because they are deterministic
input-to-output invariants. The gateway-execution crate check and targeted test
suite are goal-based checks for trait-routed integration.

## Acceptance Criteria

- [x] Distillation does not persist blank or custom entity types to the graph.
- [x] Distillation does not persist free-form relationship types to the graph.
- [x] A relationship whose endpoint is not an extracted or existing entity is
  omitted without creating an `unknown` stub.
- [x] In-session entity and endpoint resolution uses normalized names, avoiding
  casing-only duplicates.
- [x] Model- or caller-supplied graph properties cannot set scope, ontology,
  taxonomy, hierarchy, epistemic, confidence, or governance-selection control
  metadata; structured ingestion receives Ward scope from trusted tool context.
- [x] Ward artifact indexing replaces artifact-authored scope metadata with the
  Ward selected by the host execution context.
- [x] Bulk Engram admission validates the complete batch before mutation, so a
  rejected custom predicate cannot leave its endpoint entities persisted.
- [x] Entity IDs are atomically claimed by one agent and cannot be reassigned by
  concurrent, direct, or bulk graph writes.
- [x] Administrative and recursive Ward indexing never follows symlinked Ward
  roots or subdirectories.
- [x] Distilled memory facts retain their existing persistence path.
- [x] Targeted gateway-execution tests and package type-check pass.

## Assumptions

- Technical: `SessionDistiller` writes entities and relationships through
  `KnowledgeGraphStore`; the Engram adapter maps that trait to provider writes
  (source: `gateway/gateway-execution/src/distillation.rs`; `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`).
- Technical: the domain exposes built-in entity and relationship classifications
  and represents unsupported extracted labels as `Custom` (source:
  `services/knowledge-graph/src/types.rs`).
- Product: forward-only governance preserves facts and omits ungoverned graph
  candidates; historical graph repair is excluded (source: user confirmation
  2026-08-07).
- Process: the adapter-first boundary keeps product policy in z-Bot and generic
  graph primitives in Engram (source: `docs/rfc/0011-engram-memory-engine-cutover.md`).
