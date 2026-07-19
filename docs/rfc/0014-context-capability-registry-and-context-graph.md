# RFC-0014: Context Capability Registry And Context Graph

- **Status:** Draft
- **Author:** zbot maintainers
- **Approver:** phanijapps
- **Date opened:** 2026-07-06
- **Date closed:**
- **Related:** `docs/rfc/0009-agent-runtime-budget-and-retry-governance.md`; `docs/rfc/0010-extract-runtime-compaction-crate.md`; `docs/rfc/0011-engram-memory-engine-cutover.md`; `docs/specs/rig-engine-migration/spec.md`; `docs/specs/engram-memory-engine-cutover/spec.md`; `docs/specs/dynamic-ontology-skos-taxonomy/spec.md`; `docs/rfc/0014-notes/zbot-context-graph-tools-survey.md`; `docs/rfc/0014-notes/zbot-tools-memory-context-current-future-report.md`

## The ask

Approve a new zbot capability model that stops exposing every readable thing as
a model-visible tool. The recommendation is to build a zbot-owned Context
Capability Registry, route readable context through resources and context graph
packets, keep durable memory/knowledge behind Engram-backed stores, and reserve
model-visible tools for actions.

The current runtime already has useful memory, graph, recall, skill, ward,
connector, MCP, and tool-result machinery. The complication is that these
surfaces are fragmented: the model calls broad tools such as `memory`,
`graph_query`, `query_resource`, `load_skill`, and `shell` to discover context
that zbot could assemble before the model call. The question is whether to keep
adding tools, move everything to Rig/MCP/Engram shapes directly, or introduce a
stable zbot capability boundary that simplifies the prompt surface while
preserving gateway/UI contracts.

Decisions requested:

1. Create a Context Capability Registry.
   Recommended: accept. Default if no objection by 2026-07-09: one zbot-owned
   registry emits executable tools, readable resources, context graph packet
   builders, and UI/API capability metadata.
2. Treat the context graph as a derived context assembly layer.
   Recommended: accept. Default if no objection by 2026-07-09: context graph
   packets derive from Engram-backed memory/knowledge plus execution logs,
   artifacts, resources, skills, sessions, and runtime state. They do not
   become a third durable truth store.
3. Make ingestion the internal evidence intake boundary.
   Recommended: accept. Default if no objection by 2026-07-09: all durable
   memory and knowledge writes flow through a shared ingestion/distillation
   pipeline, while the model still sees separate action/resource surfaces. The
   previous SQLite memory/knowledge provider is not a terminal fallback;
   `conversations.db` may stay SQLite, but semantic memory and knowledge move
   behind Engram-backed adapter contracts.
4. Retire, hide, or split obsolete model-visible tools.
   Recommended: accept. Default if no objection by 2026-07-09: retire
   discovery/UI-only tools from the default prompt, split broad tools, then
   delete or quarantine their model-visible registrations after journey parity.
   Keep action tools such as `delegate_to_agent`, `wait_agent`, `write_file`,
   and `edit_file`.
5. Keep `wait_agent` as a parallel gateway join primitive.
   Recommended: accept. Default if no objection by 2026-07-09: `wait_agent` is
   preserved for BPMN-style parallel joins, hidden from sequential flows, and
   potentially renamed or wrapped later as `join_agents`.

## Problem & goals

The problem is not that zbot lacks memory, graph, or tools. The problem is that
the model-visible surface does not match the actual semantics of the system:

- Some capabilities are actions with side effects: shell, file writes,
  delegation, ingestion, connector invocation, agent control.
- Some capabilities are read-only context: memory facts, graph neighborhoods,
  skill docs, connector GET resources, prior tool results, artifacts, ward
  metadata, prior episodes.
- Some capabilities are runtime services: recall, micro-recall, distillation,
  title derivation, context compaction, tool-result offload, skill selection.
- Some capabilities are product/UI metadata: tool inventory, health, risk,
  actor availability, token cost, traceability.

Today those categories are mixed into broad tools. The AAPL conversation DB
fixture shows the cost: `shell` was called 61 times, `read` 38 times,
`load_skill` 6 times, and `grep` only once. Many shell calls were file
discovery, grep, JSON extraction, and validation work that should be structured
resources or bounded context packets. Several skill loads were large enough to
offload to temp files, after which the agent spent more tool calls inspecting
the offloads.

Goals:

- Reduce prompt tokens and tool calls by assembling context before the model is
  called.
- Preserve gateway HTTP routes, WebSocket event shapes, UI contracts, current
  conversation DB parity fixtures, and actor tool policy.
- Keep hard runtime actor enforcement; never rely on prompt text to enforce
  security.
- Keep Engram as the durable memory/knowledge framework while zbot owns product
  policy, ontology, taxonomy, sleep-cycle scheduling, cleanup, and UI read
  models.
- End with no production memory/knowledge behavior routed through the previous
  SQLite memory provider. SQLite remains allowed for `conversations.db`,
  execution state, and outbox/replay concerns only.
- Make recall, micro-recall, and distillation structured and observable.
- Keep the execution engine interchangeable: legacy, Rig-backed, or future
  engines all consume the same zbot capability boundary.
- Give the UI an authoritative capability catalog instead of placeholder
  `/api/tools` responses.
- Retire obsolete prompt-visible discovery and UI metadata tools without
  deleting compatibility paths prematurely.

Non-goals:

- No immediate deletion of existing tool implementations before parity gates
  pass; the terminal migration still deletes or quarantines obsolete
  model-visible registrations.
- No gateway/UI route or DTO break in the first implementation spec.
- No replacement of Engram with a separate context graph database.
- No move of zbot ontology/taxonomy policy into Engram.
- No requirement that the model manually call a tool for every read-only
  context lookup.
- No removal of `wait_agent`; it remains a parallel join primitive.
- No claim that MCP, Rig, or Engram alone defines zbot's product-level
  capability contract.
- No deletion of the SQLite conversation DB contract.

## Proposal

### 1. Context Capability Registry

Add a zbot-owned registry that is the single policy-controlled source for:

- executable tools
- read-only resources
- context graph packet builders
- UI/API-visible capability metadata

Each capability entry should carry:

- `id`
- `kind`
- `display_name`
- `description`
- `actor_policy`
- `risk_level`
- `side_effects`
- `input_schema`
- `output_schema`
- `cost_hint`
- `latency_hint`
- `token_hint`
- `resource_uri_template`
- `result_envelope`
- `health`
- `owner_crate`
- `audit_policy`

The registry describes and routes capabilities. It does not replace hard
runtime enforcement. The existing gateway actor filters remain the enforcement
layer for root, delegated executor, delegated reviewer, and ward agents.

The registry should emit four views:

1. `tools`: model-callable actions.
2. `resources`: read-only context providers with stable handles.
3. `context_graph`: bounded graph expansion and packet assembly.
4. `capability_catalog`: UI/API metadata for tool monitor, Observatory, Memory,
   Graph, and future debug screens.

### 2. Tool and resource boundary

Model-visible tools should be actions:

- `shell`
- `read` for explicit reads
- `write_file`
- `edit_file`
- `delegate_to_agent`
- `wait_agent` for parallel joins
- `respond`
- `update_plan`
- `ingest`
- `memory_write`
- `run_procedure`
- side-effecting connector actions
- explicit runtime controls such as `kill_agent` and `steer_agent` when the
  journey requires live intervention

Read-only context should enter through resources or context packets:

- memory facts
- beliefs and contradictions
- graph entities and neighborhoods
- file tree, search, and selected snippets
- skill summaries and section handles
- MCP server manifests and read-only resources
- connector GET resources
- prior tool result handles
- artifacts
- wards
- procedures
- prior episodes
- research source handles

Compatibility wrappers can keep old names available during migration, but the
default prompt-visible surface should become smaller and more action-oriented.

### 3. Context graph packets

A context graph packet is a bounded, traceable read model prepared before a
model call. It is built from durable memory/knowledge plus runtime/session
state. It is not a new durable source of truth.

The initial packet shape:

```text
ContextPacket
  request_id
  agent_id
  conversation_id
  ward_id
  actor_kind
  budget
  atoms[]
  graph_nodes[]
  graph_edges[]
  resource_handles[]
  tool_result_handles[]
  dropped[]
  trace
```

Each `ContextAtom` should carry:

```text
ContextAtom
  id
  kind
  content
  score
  confidence
  source
  source_id
  provenance
  valid_from
  valid_until
  visibility
  route_hint
  token_estimate
  render_policy
```

The context build path should:

1. Read task state, actor kind, active ward, agent config, active goals, recent
   tool results, and user request.
2. Use intent/query gates to decide which retrieval lanes are needed.
3. Retrieve facts, beliefs, procedures, wiki entries, graph seeds, graph
   traversal hits, episodes, skills, resources, and artifacts.
4. Link entities and bounded taxonomy concepts.
5. Expand a small confidence-aware graph around the task.
6. Fuse and rerank with existing recall machinery: category weights, ward
   boosts, temporal decay, contradiction penalties, supersession filtering,
   RRF, MMR, and route hints.
7. Budget by lane before rendering.
8. Render concise model text with stable handles.
9. Emit a context build trace for observability.

### 4. Ingestion, distillation, and durable memory/knowledge

`ingest` should become the canonical internal evidence intake boundary for
durable memory and knowledge. The model should not use one broad tool for every
memory operation; instead, zbot should use one internal pipeline for every
durable memory/knowledge write.

Evidence sources:

- conversations
- explicit `memory_write`
- tool results
- resource reads worth preserving
- artifacts
- connector data
- structured graph payloads
- skill/procedure indexes
- ward files
- human UI edits

Pipeline:

```text
evidence
  -> normalize/redact/classify
  -> extract candidates
  -> validate schema, scope, ontology, taxonomy, endpoints, provenance
  -> score confidence and retention value
  -> commit memory facts, graph nodes, graph edges, episodes, artifacts,
     beliefs, procedures, summaries, and distillation run stats
  -> consolidate through sleep-cycle workers
```

The public action distinction remains:

- `memory_write`: "store this explicit fact."
- `ingest`: "process this evidence and derive memory/knowledge."
- `resource_read`: "read context without persisting by default."
- `context_graph`: "retrieve assembled context."

Terminal state: these writes land through Engram-backed zbot adapter contracts.
The old SQLite `memory_facts`, `knowledge.db`, sqlite-vec, and direct
`KnowledgeDatabase`/`MemoryRepository` production path may exist only as
migration source readers or tests until those fixtures are retired. They are not
a runtime fallback once cutover is accepted.

### 5. Recall and micro-recall

Unified recall should return structured `ContextAtom`s and trace metadata, not
only prompt text. Existing strengths stay: query gating, hybrid memory search,
high-confidence facts, corrections, ward boosts, procedure recall, graph ANN,
confidence-weighted traversal, previous episodes, active goals, beliefs,
hierarchy LCA paths, RRF, and MMR.

Micro-recall should produce `ContextPacketDelta`s. Existing triggers stay:
pre-delegation, tool error, ward entry, and entity mention. New triggers should
include file write/edit completion, connector resource read, high-risk tool
result, contradiction signal, user correction, unresolved plan step, and
delegation callback.

### 6. Tool retirement and hiding policy

Old tools are discovery/UI-only tools and broad context-pull tools that make the
model assemble context manually. They are not the same thing as retained action
tools. `wait_agent`, `delegate_to_agent`, `respond`, `read`, `write_file`,
`edit_file`, and other explicit action tools remain.

Retire from default model-visible surface, then delete model-visible
registration after parity:

- `list_tools`
- `list_skills`
- `list_mcps`
- `set_session_title`
- legacy `write` / `edit` aliases
- `todo`
- `glob` after file-index resources exist

Split or wrap before hiding broad tools:

- `memory`
- `query_resource`
- `graph_query`
- `grep`
- `shell`
- `ward`
- `load_skill`

Compatibility wrappers are temporary migration scaffolding. They should not
survive the terminal cleanup except as non-model internal services, migration
readers, or explicitly named admin/debug endpoints that are not offered to the
model.

Keep as actions:

- `read`
- `write_file`
- `edit_file`
- `delegate_to_agent`
- `wait_agent`
- `respond`
- `update_plan`
- `ingest`
- `run_procedure`
- `multimodal_analyze` when needed
- `kill_agent`
- `steer_agent`
- `handoff_to_agent`

Candidate redesign/quarantine:

- `execution_graph`
- `python`
- `web_fetch`
- `request_input`
- `show_content`

`set_session_title` should move to a runtime title service that derives and
updates the title from explicit user title, intent output, first user message,
or first meaningful plan/result. It should preserve the UI event contract but
not consume a model-visible tool call.

`load_skill` should remain but return a bounded skill packet: summary, relevant
sections, section handles, token estimate, and a resource URI. Full skill body
reads should be explicit debug/admin or section-level resource reads.

`wait_agent` should remain because it models a BPMN-style parallel gateway: a
parent execution starts multiple children and joins required branches before
synthesis. It should not be used for ordinary sequential delegation.

### 7. Migration sequence

1. Add the read-only capability catalog and wire `/api/tools` from it.
2. Standardize result envelopes for tools and resources.
3. Define `ContextAtom`, `ContextPacket`, resource handles, tool result
   handles, and traces.
4. Convert unified recall output into typed atoms before rendering.
5. Add context graph assembly over memory, KG, episodes, resources, tools,
   artifacts, skills, agents, wards, and active state.
6. Convert micro-recall output to packet deltas.
7. Route skill and file/code context through resources.
8. Split connector resources from connector actions.
9. Move title generation out of the tool list.
10. Hide obsolete default tools and keep compatibility/debug paths.
11. Build journey fixtures from the current conversation DB and synthetic MCP
    journeys.
12. Delete obsolete model-visible registrations, prompt/template mentions, and
    stale UI/debug assumptions after fixtures pass.
13. Remove the previous SQLite memory/knowledge provider from production
    memory, recall, graph, hierarchy, belief, and sleep-cycle paths while
    preserving the SQLite conversation DB contract.

## Options considered

The option space is MECE along the axis of **where zbot defines model-facing
capabilities and context selection**.

| Option | Description | Trade-off |
| --- | --- | --- |
| Do nothing | Keep broad model-visible tools and current recall/working-memory rendering. | No implementation cost, but prompt/tool-call waste continues and tool semantics remain muddy. |
| Add more narrow tools | Split `memory`, `graph_query`, `query_resource`, and skill/file helpers into many model-visible tools. | More explicit than today, but still makes the model discover and assemble context manually. Tool count grows and prompt instructions get heavier. |
| Adopt MCP shapes directly | Treat MCP tools/resources/prompts as the primary zbot capability contract. | Strong external alignment, but zbot still needs actor policy, Engram-backed memory, gateway/UI contracts, and non-MCP first-party capabilities. Rig path also still has MCP lifecycle gaps. |
| Push everything through Engram | Let Engram own memory, graph, context retrieval, taxonomy, ontology, and consolidation directly. | Good durable substrate, but leaks zbot product policy and gateway/UI contract into the memory framework. |
| **Context Capability Registry + context graph packets** | zbot owns capability policy and context assembly; tools/resources/MCP/Engram/Rig plug into that boundary. | Adds a registry and packet layer, but creates a stable simplification point and preserves product contracts. Recommended. |

The recommended option is the registry and packet layer because it is the only
option that addresses tool count, token budget, runtime policy, UI metadata,
Engram integration, and engine independence together.

## Risks & what would make this wrong

Pre-mortem:

- The registry becomes a second tool system instead of a catalog. Mitigation:
  keep existing tool execution path, wrap it, and make the registry metadata
  first.
- Context packets lose critical detail and reduce answer quality. Mitigation:
  keep raw resource handles, selected/dropped traces, and current DB journey
  parity fixtures.
- Security policy drifts into descriptions. Mitigation: actor filters remain
  hard runtime enforcement; registry metadata is descriptive, not authoritative
  for access.
- Engram capabilities do not cover enough graph/memory reads. Mitigation:
  fail closed through capability reports and keep adapter sidecars until Engram
  ports exist.
- Tool retirement breaks existing prompts. Mitigation: hide from default
  catalog first; keep compatibility aliases until journey parity proves removal
  safe.
- Compatibility aliases linger after parity and recreate the old tool surface.
  Mitigation: terminal cleanup has repository-search gates and treats lingering
  model-visible registrations as failures.
- The old SQLite memory provider remains as a silent fallback. Mitigation:
  Engram mode fails closed; after cutover the only allowed SQLite production
  concerns are conversations, execution state, and outbox/replay.
- `wait_agent` gets overused as polling. Mitigation: expose it only in
  parallel-join journeys and prefer callback/result handles for sequential
  delegation.
- Context graph construction adds latency. Mitigation: bounded retrieval,
  cached resource summaries, token budgets, and observable timing metrics.

Falsifiable assumptions:

- The largest prompt waste comes from model-pulled context discovery, broad
  tool results, and skill/file offload inspection.
- Existing recall machinery can be adapted to `ContextAtom`s without changing
  the public memory/search contract.
- `/api/tools` can become a catalog view without changing execution semantics.
- Engram can remain the durable memory/knowledge framework while zbot owns
  product context assembly.
- A context packet can preserve or improve answer quality when raw data stays
  available through handles.

Drawbacks:

- The registry and packet layer is another abstraction to design, test, and
  document.
- The first implementation will temporarily increase code paths because old
  tools and new resources coexist.
- Debugging requires good traces; otherwise dropped context will be hard to
  reason about.
- Some agents may initially under-call tools because context arrives earlier
  and the prompt-visible surface is smaller.

## Evidence & prior art

Spike result:

- The current conversation DB has 8 sessions, 8 executions, 307 messages, 342
  execution logs, 13 artifacts, and 8 successful distillation runs.
- The AAPL fixture used `shell` 61 times, `read` 38 times, `load_skill` 6
  times, and `grep` once. Many shell calls were file discovery, grep, Python
  snippets for JSON inspection, and validation.
- Skill loads generated large offloaded results that were then inspected via
  additional reads and shell commands.
- The DB has no MCP-backed execution, so MCP coverage needs a synthetic e2e
  fixture.

Repo precedent:

- `runtime/agent-runtime/src/tools/registry.rs` is a simple in-memory registry
  without policy metadata, health, resource views, or UI inventory.
- `gateway/gateway-execution/src/invoke/executor.rs` already has hard actor
  capability filters and conditional tool registration.
- `runtime/agent-tools/src/tools/memory.rs` mixes key-value memory, fact
  writes, recall, exact ctx reads, beliefs, and contradictions.
- `runtime/agent-tools/src/tools/graph_query.rs` exposes rich graph context but
  as a model-pulled tool.
- `runtime/agent-tools/src/tools/connectors.rs` mixes connector discovery,
  read-only resource fetches, and side-effecting invocation.
- `gateway/gateway-memory/src/recall/mod.rs` already fuses facts, wiki,
  procedures, graph nodes, traversal hits, episodes, goals, beliefs, hierarchy
  paths, RRF, and MMR.
- `gateway/gateway-execution/src/invoke/micro_recall.rs` already performs
  event-triggered recall after tool results.
- `runtime/agent-runtime/src/context_management.rs` and RFC-0009 already
  establish prompt-safe tool-result offload and replay reduction.
- `gateway/src/http/tools.rs` is currently a placeholder, confirming no
  authoritative public capability inventory exists.
- RFC-0011 keeps Engram behind zbot-owned adapter/contracts, which this RFC
  follows.

External prior art:

- [Model Context Protocol](https://modelcontextprotocol.io/specification/2025-06-18)
  separates tools, resources, and prompts; this RFC adopts that distinction
  while keeping zbot's product policy above it.
- [MCP tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
  support structured results and output schemas, which informs result
  envelopes.
- [MCP resources](https://modelcontextprotocol.io/specification/2025-06-18/server/resources)
  model read-only context as URI-addressed resources, matching the proposed
  resource lane.
- [Anthropic context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
  emphasizes finite context budgets, compaction, tool-result clearing,
  structured notes, and layered context loading.
- [Anthropic effective agents](https://www.anthropic.com/engineering/building-effective-agents)
  emphasizes simple agent designs and clear tool interfaces.
- [Microsoft GraphRAG](https://microsoft.github.io/graphrag/) uses graph
  structure to assemble context for global and local questions, supporting the
  context graph packet direction.
- [Graphiti/Zep](https://help.getzep.com/graphiti/getting-started/overview)
  treats temporal context graphs as dynamic agent memory, supporting temporal
  and provenance-aware context.
- [PuppyGraph context graph](https://www.puppygraph.com/blog/context-graph)
  is useful vocabulary for context graphs as operational, provenance-bearing
  graph context, but it is vendor prior art and is not treated as the sole
  authority.

## Experiment / validation

Validate this RFC with a staged implementation spec.

Hypothesis:

- A registry plus context packet path reduces default prompt-visible tools and
  model-pulled context calls without reducing answer parity on current
  conversation fixtures.

What we measure:

- Tool calls per turn on AAPL fixture replay.
- Prompt token estimate before and after packet rendering.
- Count of shell calls used for file discovery, grep, JSON extraction, and
  validation.
- Number of selected vs. dropped context atoms.
- Recall hit rate and manual correction rate.
- Context packet latency.
- Parity of gateway/UI events and artifacts.
- Whether synthetic MCP journey can discover MCP resources through the catalog
  without exposing raw discovery tools.

Success criteria:

- `/api/tools` returns a non-empty actor-filtered capability catalog.
- AAPL journey replay can be represented as context packets and action tools
  without broad skill/offload inspection loops.
- `set_session_title` is replaced by runtime title derivation without UI
  regression.
- `wait_agent` remains available for parallel joins.
- Obsolete tools are first hidden from the default catalog, then removed from
  model-visible registration after parity. Any remaining access is explicitly
  non-model internal/admin surface.
- The previous SQLite memory/knowledge provider is not reachable from
  production memory, recall, graph, hierarchy, belief, or sleep-cycle paths.

Failure criteria:

- Actor policy depends on prompt instructions.
- Raw resource handles leak secrets or inaccessible data.
- Context packet rendering loses required facts without traceable dropped-item
  diagnostics.
- The Rig path or MCP lifecycle regresses.
- Current gateway/UI contracts change without a separate accepted spec.
- Obsolete tools or the previous SQLite memory provider remain reachable in the
  normal model/runtime path after terminal cleanup.

## Open questions

1. **Should the implementation start in `gateway-memory`/`gateway-execution` or
   a new crate?** Recommended default: start at existing gateway boundaries and
   extract after packet/catalog shapes stabilize; owner: zbot maintainer;
   decide-by: T1 of the follow-on spec.
2. **What is the initial context packet token budget?** Recommended default:
   8k for normal sessions, 16k for research/code sessions, with per-lane caps;
   owner: runtime maintainer; decide-by: T3 of the follow-on spec.
3. **Should `wait_agent` be renamed to `join_agents`?** Recommended default:
   keep `wait_agent` for compatibility and add catalog metadata naming it a
   parallel join; owner: gateway maintainer; decide-by: before hiding obsolete
   default tools.

## Follow-on artifacts

- Spec: `docs/specs/context-capability-registry/`
- Possible ADR after acceptance: "Zbot model-facing capabilities are tools,
  resources, and context packets."
- Possible contract files after implementation design settles:
  `contracts/jsonschema/context-capability-catalog.schema.json` and
  `contracts/jsonschema/context-packet.schema.json`
