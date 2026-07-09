# Spec: Context Capability Registry

- **Status:** Implementing
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [`RFC-0014`](../../rfc/0014-context-capability-registry-and-context-graph.md); [`RFC-0011`](../../rfc/0011-engram-memory-engine-cutover.md); [`rig-engine-migration`](../rig-engine-migration/spec.md); [`engram-memory-engine-cutover`](../engram-memory-engine-cutover/spec.md); [`dynamic-ontology-skos-taxonomy`](../dynamic-ontology-skos-taxonomy/spec.md)
- **Brief:** none
- **Contract:** [`contracts/jsonschema/context-capability-catalog.schema.json`](../../../contracts/jsonschema/context-capability-catalog.schema.json); [`contracts/jsonschema/context-packet.schema.json`](../../../contracts/jsonschema/context-packet.schema.json)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Build the first implementation of RFC-0014 by adding a zbot-owned Context
Capability Registry and structured context packet path that reduce
model-visible tool clutter without changing gateway/UI contracts. Success means
zbot can expose an actor-filtered capability catalog, assemble bounded
`ContextPacket`s from existing recall/resource/tool metadata, move
model-visible discovery/UI-only tools out of the default prompt, preserve
`wait_agent` as a parallel-join action, and validate the AAPL conversation DB
journey plus a synthetic MCP/research journey before broad tool retirement.
Terminal success also means obsolete model-visible tool registrations are gone
after parity and the previous SQLite memory/knowledge provider is no longer a
production runtime fallback.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep actor capability policy enforced in gateway/runtime code; catalog
  metadata may describe access but must not enforce it.
- Preserve existing gateway route paths, WebSocket event shapes, UI DTOs,
  conversation DB shape, and tool execution semantics in the first slice.
- Treat Engram-backed memory/knowledge stores as durable truth and the context
  graph packet as a derived read model.
- Preserve SQLite only for non-semantic-memory concerns that are already part
  of the product contract: `conversations.db`, execution state, bridge outbox,
  replay/migration fixtures, and tests.
- Keep zbot-owned dynamic ontology and SKOS taxonomy policy above Engram and
  pass only selected policy into memory/knowledge ingestion.
- Keep `wait_agent` available for BPMN-style parallel joins.
- Hide or split tools in stages: first catalog metadata, then compatibility
  wrappers, then default prompt removal after journey parity, then delete or
  quarantine obsolete model-visible registrations.
- Keep raw tool results, resource reads, artifacts, and full skill text behind
  handles when not rendered inline.
- Record selected and dropped context in traces so bad recall can be debugged.

### Ask first

- Renaming public tools or deleting tool implementations before the terminal
  parity gate.
- Changing public `/api/tools`, Memory, Graph, Observatory, Vault, Research, or
  WebSocket payload shapes beyond additive fields.
- Moving zbot sleep-cycle scheduling, cleanup, ontology, taxonomy, or UI read
  model ownership into Engram.
- Exposing raw MCP connector data, tool-result files, full skill bodies, or
  private absolute paths in model-visible context by default.
- Renaming `wait_agent` to `join_agents` or changing its arguments/results.

### Never do

- Never rely on prompt text to enforce actor policy, resource visibility, or
  security-sensitive tool access.
- Never make the context graph a separate durable source of truth competing
  with Engram-backed memory/knowledge and execution logs.
- Never remove sequential delegation callbacks/result handles and replace them
  with polling.
- Never expose credentials, connector secrets, raw private transcripts,
  embeddings, local database contents, or private file paths through the
  catalog or context packet.
- Never mark an obsolete tool removed until the AAPL fixture and synthetic
  MCP/research fixture pass with compatibility paths still available.
- Never leave the previous SQLite memory/knowledge provider as a production
  fallback after Engram parity and terminal cleanup; `conversations.db` is the
  explicit exception because it is conversation/execution history, not semantic
  memory.

## Testing Strategy

- Capability catalog shape and actor filtering: **TDD**. The output is a pure
  contract-shaped value with compact invariants.
- Context packet assembly, scoring, budgeting, selected/dropped tracing, and
  render policy: **TDD plus integration tests** over existing recall outputs and
  synthetic resource/tool-result handles.
- Gateway `/api/tools` catalog wiring: **goal-based integration** because the
  behavior spans gateway state, actor policy, and JSON schema shape.
- Tool visibility migration: **goal-based integration** using registry snapshots
  before/after default hiding.
- Terminal cleanup: **goal-based repository checks** proving obsolete
  model-visible tool names and old SQLite memory/knowledge providers are absent
  from production registration and prompts, with only documented migration/test
  allowlist hits.
- AAPL and MCP/research journeys: **goal-based e2e fixtures**. AAPL comes from
  current `~/Documents/zbot/data/conversations.db`; MCP/research is synthetic
  because the current DB has no MCP-backed run.
- UI and Observatory visibility: **manual QA plus existing UI tests**. The
  initial spec only requires additive catalog/trace visibility and no route
  regression.

## Acceptance Criteria

- [x] A `ContextCapabilityCatalog` domain type serializes to
  `contracts/jsonschema/context-capability-catalog.schema.json` and includes
  actor kind, capability id, kind, display name, actor policy, risk level, side
  effects, schemas, hints, health, owner crate, and audit policy.
- [x] The catalog is populated from the existing first-party tool registry,
  gateway actor capability filters, MCP manager metadata when configured,
  connector resource provider metadata, memory/graph/recall services, and
  resource providers without changing tool execution behavior.
- [x] `/api/tools` and `/api/tools/:name` return actor-filtered catalog data
  instead of placeholder empty/404 responses while preserving existing route
  paths.
- [x] A `ContextPacket` domain type serializes to
  `contracts/jsonschema/context-packet.schema.json` and carries request/session
  identity, actor kind, budget, atoms, graph nodes, graph edges, resource
  handles, tool result handles, dropped candidates, and trace metadata.
- [x] Unified recall can emit `ContextAtom`s with score, confidence,
  provenance, validity, visibility, route hint, token estimate, and render
  policy before any prompt text is produced.
- [x] Micro-recall can emit a `ContextPacketDelta` for at least tool errors,
  ward entry, pre-delegation, delegation callback, and entity mention without
  directly appending unstructured markdown to working memory.
- [x] Skill loading returns a bounded skill packet with summary, relevant
  sections, section handles, token estimate, and resource URI; full skill body
  reads require an explicit section/debug resource read.
- [x] Session title derivation is available as a runtime service and can update
  `sessions.title` from explicit title, intent title hint, first user message,
  or first meaningful plan/result without exposing `set_session_title` in the
  default model-visible catalog.
- [x] `wait_agent` remains cataloged as an action for parallel joins and is
  hidden from ordinary sequential journey prompts unless multiple active child
  executions require a join.
- [x] Discovery/UI-only tools (`list_tools`, `list_skills`, `list_mcps`,
  `set_session_title`, `todo`, legacy `write`/`edit`, and eventually `glob`)
  are hidden from the default model-visible catalog during migration and their
  model-visible registrations are deleted or quarantined after journey parity.
- [ ] Broad tools (`memory`, `query_resource`, `graph_query`, `shell`,
  `ward`, `load_skill`) have catalog metadata naming the split target and
  default visibility policy before any default hiding lands; after parity,
  broad context-pull wrappers are removed from default model registration or
  converted into non-model internal services/resources. (deferred:
  context-capability-broad-tool-split-completion)
- [x] The ingestion path records a single internal evidence intake boundary for
  durable memory/knowledge writes, with separate model-facing actions for
  `memory_write`, `ingest`, resource reads, and context graph retrieval.
- [x] Previous SQLite memory/knowledge production paths are gone after terminal
  cleanup: no production memory, recall, graph, hierarchy, belief, or sleep
  worker path constructs or depends on `KnowledgeDatabase`, `MemoryRepository`,
  `GatewayMemoryFactStore`, `SqliteMemoryStore`, `SqliteKgStore`, sqlite-vec
  memory indexes, `knowledge.db`, or `memory_facts` except migration readers,
  tests, and the preserved non-memory `conversations.db` contract.
- [x] AAPL fixture replay demonstrates a context-packet/action-tool journey
  from the current conversation DB and records baseline vs. target counts for
  shell, read, load-skill, and context packet tokens.
- [x] Synthetic skills + MCP + code + research fixture demonstrates MCP
  discovery through catalog metadata and read-only resources rather than
  model-visible raw discovery tools.
- [ ] Existing gateway execution, Rig adapter, memory/graph/Observatory,
  Vault/Research, and UI tests continue to pass, or the spec is updated with an
  explicit accepted contract change.

## Assumptions

- Technical: `ToolRegistry` is currently an in-memory first-party registry
  without durable policy metadata or UI inventory semantics (source:
  `runtime/agent-runtime/src/tools/registry.rs`).
- Technical: gateway execution already hard-filters first-party tools by actor
  capability and must remain the enforcement layer (source:
  `gateway/gateway-execution/src/invoke/executor.rs`).
- Technical: `/api/tools` currently returns an empty list and 404 detail because
  it is not wired to the runtime registry (source: `gateway/src/http/tools.rs`).
- Technical: unified recall already fuses memory facts, wiki, procedures,
  graph hits, traversal, episodes, goals, beliefs, hierarchy paths, RRF, and
  MMR (source: `gateway/gateway-memory/src/recall/mod.rs`).
- Technical: the current conversation DB contains AAPL and XOM valuation
  sessions but no MCP-backed execution, so MCP journey coverage must be
  synthetic first (source: read-only SQLite query against
  `~/Documents/zbot/data/conversations.db` on 2026-07-06).
- Product: memory is omnipresent and should not be drawn as a separate journey
  lane; it runs through recall, context build, micro-recall, and distillation
  throughout every journey (source: user confirmation 2026-07-06).
- Product: `wait_agent` is a BPMN-style parallel gateway join primitive and
  should be kept, not retired (source: user confirmation 2026-07-06).
- Product: the previous SQLite memory layer should be completely gone after
  migration; the preserved SQLite exception is the conversation/execution DB
  contract, not semantic memory/knowledge (source: user confirmation
  2026-07-06).
- Process: cross-cutting capability and tool-surface changes belong in an RFC
  before implementation specs (source: `docs/CONVENTIONS.md`).
