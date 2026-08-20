# Plan: Distillation Graph Governance

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Harden the distillation projection before it invokes the existing graph-store
trait. Candidate entities and relationships receive deterministic validation;
only built-in classifications and resolved endpoints reach the adapter. The
change remains forward-only and preserves the existing memory-fact flow.

## Constraints

- RFC-0011 retains z-Bot ownership of product policy and the adapter boundary.
- No direct SQLite access or historical graph mutation is permitted.

## Construction tests

**Integration tests:** package tests exercise the trait-routed graph path.
**Manual verification:** none; this is an internal persistence policy change.

## Design (LLD)

### Data & schema

The Engram compatibility sidecar adds `kg_entity_owners(id PRIMARY KEY,
agent_id)` for atomic ownership claims. Store startup creates the table and
backfills existing `kg_entities` rows with `INSERT OR IGNORE`. Distillation
still filters candidates before calling `KnowledgeGraphStore`.

### Behavior & rules

- A blank name or `Custom` entity type is evidence-only and is omitted from the
  graph projection.
- A `Custom` relationship type is evidence-only and is omitted from the graph
  projection.
- Endpoint resolution reuses only entities from the current candidate set or
  the existing graph; it never creates `unknown` entities.
- Candidate names are normalized for in-session lookup.
- LLM entity properties are copied only after reserved adapter control fields
  (scope, governance, hierarchy, epistemic, and confidence) are removed.

### Failure, edge cases & resilience

Omitted candidates are logged as structured diagnostics. A graph-store failure
remains best-effort and does not fail fact distillation.

## Tasks

### T1: Govern distillation graph candidates

**Status:** Done on 2026-08-07.

**Depends on:** none

**Touches:**

- `gateway/gateway-execution/src/distillation.rs`
- `gateway/gateway-execution/src/invoke/ingest_adapter.rs`
- `gateway/gateway-execution/src/runner/core.rs`
- `gateway/gateway-execution/src/ward_artifact_indexer.rs`
- `gateway/src/http/graph.rs`
- `runtime/agent-tools/src/tools/ingest.rs`
- `stores/zbot-stores/src/knowledge_graph.rs`
- `stores/zbot-stores-sqlite/src/knowledge_graph.rs`
- `stores/zbot-engram-adapter/src/stores/knowledge_graph.rs`
- `stores/zbot-engram-adapter/tests/knowledge_graph.rs`

**Tests:**
- TDD unit tests in `distillation.rs` prove blank/custom labels are rejected,
  padded built-in labels are accepted, names normalize deterministically, and
  unresolved candidates do not create in-session stubs. The SQLite-backed
  trait integration test covers case-variant reuse through the normalized
  exact-lookup contract; unit tests cover reserved adapter-control properties.
- A SQLite-backed integration test calls the projection through
  `Arc<dyn KnowledgeGraphStore>` and verifies: governed writes persist; custom
  entities and predicates do not; a normalized duplicate reuses one entity;
  unresolved endpoints never create a stub; and the sole valid edge survives.
- A recording `MemoryFactStore` test verifies distilled facts still invoke
  `upsert_typed_fact`, independently of the graph-governance filter.
- An Engram adapter integration test verifies the trait's normalized exact
  lookup resolves case- and whitespace-variant names without a ranked scan.
- Structured-ingest tests prove caller-supplied Ward and governance properties
  are removed and replaced with the Ward scope from trusted tool context.
- Ward artifact tests prove authored Ward and governance properties are removed
  and replaced with the Ward scope selected by the host runner through an
  Engram-backed indexing path.
- The ingest tool advertises only the built-in entity/predicate vocabulary and
  marks graph control properties as host-managed; an Engram-backed adapter test
  verifies unsupported custom types fail without persistence.
- The administrative reindex validates each Ward directory name and passes that
  trusted identifier as graph scope instead of collapsing wards into the
  reindex session label; symlinked Ward roots and descendants are skipped.
- Engram bulk admission preflights every entity and relationship before the
  first write, preventing rejected custom predicates from partially persisting
  otherwise valid endpoint entities.
- Engram entity admission atomically claims IDs in the sidecar before Engram
  mutation, including all IDs in a bulk batch, and rejects cross-agent
  collisions without overwriting ownership.
- Gate artifacts: `cargo test -p gateway-execution distillation --lib`,
  `cargo test -p zbot-engram-adapter --test knowledge_graph
  normalized_entity_lookup_is_exact_case_and_whitespace_insensitive`,
  `cargo check -p gateway-execution -q`,
  `cargo check -p zbot-engram-adapter -q`,
  `cargo clippy -p gateway-execution --lib -- -D warnings`, and
  `git diff --check`.

**Approach:**
- Add deterministic candidate validation ahead of trait-routed writes.
- Replace relationship endpoint stub creation with omission.
- Preserve existing fact writes and best-effort graph error handling.

**Done when:** only governed candidates call `KnowledgeGraphStore`, the
trait-routed integration test passes, and all listed gate artifacts are green.

## Rollout

The change applies to future distillation and indexing runs on deployment.
Opening the Engram graph store creates and backfills the owner-claim sidecar
table; the migration is additive and idempotent. Code rollback can leave the
unused table in place without changing historical graph records.

## Risks

- Some formerly stored free-form graph candidates are omitted. Their session
  transcript and distilled facts remain available as evidence.

## Changelog

- 2026-08-07: initial forward-only plan after graph-noise audit.
