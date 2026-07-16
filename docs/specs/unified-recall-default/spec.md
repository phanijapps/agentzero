# Spec: unified-recall-default

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0014`](../../rfc/0014-context-capability-registry-and-context-graph.md); [`dynamic-ontology-skos-taxonomy`](../dynamic-ontology-skos-taxonomy/spec.md); [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md)
- **Brief:** none
- **Contract:** [`contracts/jsonschema/unified-recall.schema.json`](../../../contracts/jsonschema/unified-recall.schema.json)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make unified recall the default on-demand and refresh retrieval path so an
agent can retrieve the best bounded blend of durable facts, graph context,
procedures, ward wiki, episodes, beliefs, hierarchy, active goals, and
taxonomy-expanded matches through one read-only `recall` tool. Users must be
able to verify each delivery phase from its observable tool output and tests.
The existing `memory(action="recall")` compatibility action also defaults to
the same unified result, while exact-key reads and historical fact lookup retain
their current semantics.

## Boundaries

### Always do

- Keep the existing `memory` wrapper hidden from the model; expose only a
  narrow read-only `recall(query, limit)` tool to actors authorized for
  `MemoryRead`.
- Treat `recall` as this spec's user-approved, bounded RFC-0014 exception: it
  requests a derived context/recall bundle, not arbitrary memory or graph
  access. Its catalog policy must explicitly name the exception.
- Use the existing `MemoryRecall` source fusion and configured dynamic SKOS
  taxonomy expander; include safe result provenance, source counts, and query
  expansion diagnostics in the new tool result.
- Treat recalled content as untrusted reference data and preserve existing
  actor-capability enforcement, scope/ward boundaries, limit bounds, and
  embedding identity safety.
- Derive a non-model-controlled authorization context for every recall through
  a gateway authorization bridge backed by the authenticated execution result.
  Bind every configured source to its authenticated agent and runtime
  tenant/workspace, pass ward scope into source queries, and apply its
  session/ward predicate to explicit provenance before RRF/MMR ranking. Missing
  provenance fails closed; only an explicit authorized `__global__` scope may
  pass. Omit unauthorized records and their provenance rather than reporting
  their existence.
- Pass the same immutable runtime scope into taxonomy expansion. A taxonomy
  provider that cannot prove the selected definition belongs to that scope
  returns a finite unavailable/not-configured status with no expansion trace.
- Apply one centralized recall-output policy before model-visible rendering:
  omit transcript-only/unauthorized candidates, redact recognised token-shaped
  secrets and private absolute paths, cap the entire response, and delimit all
  remaining content as untrusted data rather than instructions or authority.
- Retain `memory(action="get_fact")` exact reads and use fact-only historical
  lookup when an `as_of` timestamp is requested.
- Make each plan task independently verifiable before its successor begins.
- Apply configured taxonomy and ontology governance before every durable
  semantic write. Taxonomy labels are persisted classification evidence;
  ontology validation is advisory and persists only finite findings.

### Ask first

- Renaming the approved `recall` tool, exposing the broad `memory` tool, or
  changing other public model-tool names.
- Adding a user setting, new embedding/ranking dependency, database migration,
  or persistent recall-history store.
- Changing gateway REST/WebSocket payloads, AG-UI event contracts, or existing
  Context Capability JSON schema beyond the additive catalog entry for
  `recall`.
- Moving ontology/taxonomy ownership or zbot policy into Engram.

### Never do

- Never make a recall result write data, run a tool, grant authority, or bypass
  normal confirmation policy.
- Never silently represent a facts-only historical result as a unified
  historical snapshot.
- Never expose raw embeddings, private transcript bodies, connector payloads,
  absolute paths, database internals, or secrets in recall output or trace
  diagnostics.
- Never introduce ontology-based query rewriting, filtering, or ranking until
  the Phase 4 quality gate proves that it improves a curated retrieval fixture.
- Never retain a separate zbot-owned semantic store or make the tool depend on
  a concrete Engram/SQLite database type.
- Never forward raw provider, database, endpoint, stack, or path errors through
  recall diagnostics; map them to finite public reason codes and log detail
  only through existing private tracing.

## Testing Strategy

- Recall tool contract and request validation: **TDD**, because the schema,
  capability gate, limit, and historical-mode invariants are compact.
- Unified outcome projection and source diagnostics: **TDD plus integration**,
  because taxonomy expansion, source fusion, partial source degradation, and
  provenance cross the `gateway-memory` and `gateway-execution` boundary.
- Executor wiring and automatic refresh: **goal-based integration**, because
  root, continuation, delegation, and mid-session paths must agree without
  changing their public execution behavior.
- AG-UI visibility: **visual/manual QA**, exercised against a running daemon
  and UI; a forced recall call must be visible as a normal tool call with a
  source-mix result.
- Ontology readiness: **goal-based evaluation**, using a checked-in curated
  fixture and a documented before/after judgement; it is a gate, not a promise
  of ontology retrieval implementation.

## Acceptance Criteria

- [x] `contracts/jsonschema/unified-recall.schema.json` validates both the
  read-only `recall` request and response envelope, including result kind,
  bounded provenance, source status/counts, taxonomy diagnostics, and
  degraded-source notices. The contract links back to this spec.
- [x] Every actor that existing gateway policy authorizes for `MemoryRead` sees
  `recall` in its model-visible tool schema when unified recall is configured
  and healthy; the catalog reports `recall` as unavailable when it is not.
  The broad `memory`, `graph_query`, and `query_resource` tools remain hidden.
  `recall` has no write-capable arguments.
- [x] `recall(query, limit)` returns a single bounded, ranked unified result
  set from every configured available source. A missing or failing source
  produces partial results plus a named degraded/unavailable source status,
  never a fabricated result or a silent broad lexical fallback.
- [x] The gateway creates a non-forgeable recall authorization context from
  execution state rather than model arguments. Cross-ward/session,
  cross-tenant/workspace, missing-provenance, and explicit-global fixtures
  prove excluded candidates and their provenance do not enter RRF/MMR ranking
  or output.
- [x] When the configured SKOS taxonomy expands a query, the recall response
  identifies the selected concept labels/relations and the expanded retrieval
  query without crossing configured scope or visibility boundaries.
- [x] `memory(action="recall")` without `mode` or `as_of` delegates to the
  same unified result contract and preserves legacy result aliases `source`,
  `prioritized`, `recalled`, and `reason`. The historical decision table is:
  omitted `mode` plus `as_of` → facts; `mode: "facts"` plus `as_of` → facts;
  `mode: "unified"` plus `as_of` → named error. All successful historical
  responses label `mode: "facts"`; `get_fact` behavior is unchanged. The
  compatibility path uses the same gateway authorization, scoped adapter, and
  centralized output policy as `recall`.
- [x] Root sessions, continuations, delegated sessions, and the mid-session
  refresh use the same unified source semantics and generic item-ID
  deduplication. Active goals participate in ranking whenever the goal adapter
  is available.
- [x] The response envelope marks its entire result bundle as untrusted
  reference data, and injected recall retains the same trust boundary. Results
  contain no raw embeddings, secrets, private absolute paths, SQL internals, or
  private transcript bodies beyond the already authorized recalled record
  content.
- [x] Recall output applies one centralized policy: explicit hostile instruction
  text remains data under the untrusted envelope; token-shaped secrets and
  private absolute paths are redacted; transcript-only candidates are omitted;
  and the serialized response is at most 16 KiB with deterministic
  score-preserving truncation/omission and `truncated: true` when limited.
- [x] Public source and degradation diagnostics contain only finite reason
  codes. Fixtures containing database URLs, SQL errors, endpoints, or private
  paths prove those raw values do not appear in tool output.
- [x] In a running AG-UI session, the prompt "Before answering, call recall
  for 'knowledge graph memory' and list the source types you found" produces a
  visible `recall` tool call and an answer consistent with its returned source
  mix.
- [x] Phase 4 produces a checked-in ontology-retrieval evaluation fixture and
  report with at least 12 labelled queries. Every case records its scope, a
  frozen candidate-ID universe, 0/1/2 relevance grades, and baseline/candidate
  ranked IDs; the fixture records both recall configuration fingerprints. A
  deterministic test-only evaluator emits per-query and mean nDCG@5, top-1
  grades, and a pass/fail result. Ontology-aware retrieval is implemented only
  if mean nDCG@5 improves by at least 0.05 over taxonomy-plus-graph baseline
  and no query's top-1 grade falls; otherwise the report records a no-go result
  and no ontology ranking code ships. Production ontology ranking requires a
  separate approved implementation spec.
- [x] Every durable semantic producer—`memory.save_fact`/`memory_write`,
  ingestion and connector evidence, session distillation, ward/wiki indexing,
  and graph relationship writes—selects configured governance before writing
  through Engram. Persisted taxonomy classifications are available to later
  recall; ontology validation remains advisory and stores findings without
  rejecting a valid durable write.
- [x] Runtime bootstrap loads the configured confined governance definitions
  from `config/governance/`, makes the selected taxonomy expander available to
  `MemoryRecall`, and reports a finite unavailable/not-configured status rather
  than silently ignoring a configured governance source.

## Assumptions

- Technical: `MemoryRecall::recall_unified` already fuses facts, wiki,
  procedures, graph/traversal, episodes, beliefs, hierarchy, and supplied
  active goals, and calls the taxonomy expander for every unified query
  (source: `gateway/gateway-memory/src/recall/mod.rs`).
- Technical: `memory(action="recall")` currently calls only
  `MemoryFactStore::recall_facts_prioritized_scoped`, while `memory` is hidden
  from the model-visible schema (source:
  `runtime/agent-tools/src/tools/memory.rs`; `gateway/gateway-execution/src/invoke/executor.rs`).
- Technical: the current dynamic ontology/taxonomy feature makes ontology
  advisory write governance and uses SKOS labels/relations for bounded recall
  expansion; it does not make ontology a recall-ranking input (source:
  `docs/specs/dynamic-ontology-skos-taxonomy/spec.md`).
- Technical: existing model/context contracts are JSON Schema documents under
  `contracts/jsonschema/`, so a model-tool input/output envelope can use the
  same contract type (source: `docs/CONVENTIONS.md` § Contracts).
- Process: this cross-cutting tool-surface change is constrained by RFC-0014;
  the new narrow tool must preserve its actor enforcement and staged visibility
  rules, except for the explicitly approved bounded `recall` exception in this
  spec (source: `docs/specs/context-capability-registry/spec.md`; user
  confirmation 2026-07-11).
- Product: use one phased spec; approve a model-visible read-only `recall`
  tool while keeping broad `memory` hidden (source: user confirmation
  2026-07-11).
- Product: use fact-only historical lookup until non-fact sources support an
  honest historical snapshot (source: user confirmation 2026-07-11).
- Product: make ontology-aware retrieval a quality-gated final phase rather
  than asserting a benefit before it is measured (source: user confirmation
  2026-07-11).
- Security: model-visible recall must receive trusted scope from gateway
  execution state, bound output as untrusted data, and fail without exposing
  internal errors (source: pre-execution security review 2026-07-12).
