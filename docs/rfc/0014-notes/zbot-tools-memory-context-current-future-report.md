# Zbot Tools, Memory, Graph, And Context Report

## Executive Position

Zbot needs fewer model-visible tools and a stronger context assembly layer.
Today, the runtime already has serious memory and graph machinery, but the
model sees that machinery through broad tools such as `memory`, `graph_query`,
and `query_resource`. This makes the model spend tokens discovering context
that zbot can assemble before the model is called.

The future shape should be:

1. Tools are executable actions.
2. Resources are readable context.
3. Engram-backed memory and knowledge remain durable truth.
4. A zbot-owned context graph assembles small, traceable context packets.
5. Recall and micro-recall become structured context services, not just prompt
   text helpers.
6. Distillation converts raw sessions, tool events, resources, and artifacts
   into facts, graph nodes, relationships, beliefs, procedures, summaries, and
   routeable context atoms.

This is a tooling change, a memory change, and a prompt-budget change. It
should be treated as one architecture change.

## Current State

### Tool Exposure

The first-party tool registry is runtime-local. `ToolRegistry` stores a
`Vec<Arc<dyn Tool>>`, supports registration and lookup by name, and exposes no
durable metadata, risk class, resource view, actor matrix, health status, or UI
inventory contract.

Current evidence:

- `runtime/agent-runtime/src/tools/registry.rs`
- `gateway/gateway-execution/src/invoke/executor.rs`
- `gateway/src/http/tools.rs`

The gateway builds the registry per execution. It uses hard actor filters over
capabilities such as `MemoryRead`, `MemoryWrite`, `GraphRead`, `ConnectorQuery`,
`Shell`, `FileRead`, `FileWrite`, `AgentDelegate`, and `AgentControl`. This is
the right security direction: tool availability is enforced in code, not in the
prompt.

The HTTP tool endpoints exist but are placeholders. `/api/tools` returns an
empty list and `/api/tools/:name` returns `404`. That means the UI has no
authoritative capability inventory today.

### Current Tool Problems

The current model-visible tools mix actions, resources, and context lookup:

- `memory` combines key-value memory, durable fact writes, semantic recall,
  exact ctx reads, belief reads, and contradiction listing.
- `graph_query` exposes graph search, neighbor expansion, and contextual
  subgraphs as a model-called tool.
- `query_resource` combines connector discovery, read-only resource fetches,
  and side-effecting connector capability invocation.
- MCP tools are managed separately from first-party tools and connector
  resources.
- Rig adapts existing AgentZero tools into Rig's tool dispatch, but the Rig path
  still falls back when MCP servers are configured.

This creates avoidable prompt and tool-call overhead. The model has to spend
tokens asking what exists, selecting a broad action enum, and receiving raw or
semi-structured results. Zbot already knows enough to assemble much of this
context without a model-visible tool call.

### Memory

Current memory has multiple layers:

- Agent/shared JSON memory files for simple key-value entries.
- DB-backed memory facts through `MemoryFactStore`.
- Categories such as `user`, `pattern`, `domain`, `instruction`,
  `correction`, and `ctx`.
- Beliefs and contradiction checks behind optional stores.
- Hybrid semantic and keyword recall over saved facts.

`MemoryTool` is useful but too broad. It is both a write interface and a read
interface. It is also a context retrieval interface. That makes permissions,
tool descriptions, and model behavior harder to reason about.

### Knowledge Graph

`graph_query` has a cleaner shape than `memory`. It exposes:

- `search`: entity search by query/type/view.
- `neighbors`: entity-centered relationship expansion.
- `context`: topic-centered entity and relationship context.

The returned payloads already include rich entity and relationship metadata,
including ids, properties, timestamps, mention counts, and relationship
properties. That is close to the future context graph payload shape, but it is
still model-pulled rather than runtime-assembled.

The `ingest` tool already supports text ingestion and structured bulk graph
writes. Structured writes merge entities by stable id and relationships by
source, target, and type. This is a good primitive for future graph ingestion,
but the schema is currently intentionally free-form.

### Recall

Recall is stronger than the model-visible tools suggest.

`gateway-memory` already performs a unified recall pass across several sources:

- Memory facts through hybrid FTS/vector search.
- High-confidence facts.
- Corrections.
- Ward-scoped wiki articles.
- Procedures.
- Knowledge graph ANN hits.
- Confidence-weighted graph traversal from top seed entities.
- Previous ward episodes.
- Active goals.
- Beliefs.
- Hierarchical memory LCA paths and inter-cluster relations.

It applies query gating, category weights, ward boosts, temporal decay,
contradiction penalties, supersession filtering, intent boost, Reciprocal Rank
Fusion, and optional MMR diversity reranking. It also emits `RecallTrace`
events for Observatory when wired.

This is the right foundation. The issue is that the final output is still
mostly a ranked list that later becomes prompt text. The intermediate evidence
does not yet have one canonical `ContextAtom` or `ContextPacket` structure that
the runtime, UI, and model renderer all share.

### Micro-Recall

Micro-recall already does just-in-time context expansion after tool results.
It detects:

- Pre-delegation events.
- Tool errors.
- Ward entry.
- New entity mentions in tool output.

It then injects corrections, ward facts, KG entities, or error discoveries into
working memory.

This is valuable because it catches context needs that are not visible at the
start of a turn. The current limitations are:

- Entity detection is regex-based and shallow.
- Trigger output goes into working memory text, not a structured packet.
- There is no explicit provenance path from trigger to recalled fact/entity in
  the model-visible context.
- It does not yet use the same scoring and dedupe model as unified recall.

### Working Memory And Context Management

Working memory tracks active entities, discoveries, corrections, delegation
status, and pending parallel agents. It is budget-managed and rendered as a
markdown system message before each LLM iteration.

Context management already handles some token-saving mechanics:

- Large tool result offload to files.
- Tool result truncation.
- Old message compaction in tests.
- Tool/result pair sanitization.
- Summarization middleware that summarizes old prose messages while preserving
  tool messages and summaries.

These are useful pieces, but they operate below the level where context is
selected. They reduce size after results exist. The future architecture should
avoid inserting unnecessary context in the first place.

## Core Problems To Fix

### 1. Everything Looks Like A Tool

Reading memory, reading graph context, listing connector resources, fetching a
known resource, and executing shell are different operations. They should not
all compete in the same model-visible tool namespace.

The model-visible namespace should be small and action-oriented. Read-only
context should usually enter through resources or context packets.

### 2. There Is No Capability Catalog

Zbot has multiple registries and discovery paths:

- First-party `ToolRegistry`.
- Gateway actor capability filters.
- MCP manager.
- Connector resources and capabilities.
- Memory/graph recall internals.
- Placeholder HTTP tool endpoints.

There is no single source that says: what exists, who can use it, what it
costs, whether it mutates state, what schema it has, what result envelope it
returns, whether it is healthy, and how the UI should render it.

### 3. Recall Is Powerful But Not First-Class Enough

Unified recall is already doing the right hard work. It needs to become a
first-class context assembly service with typed outputs, not only a prompt text
producer.

### 4. Context Graph Is Implicit

Zbot has memory facts, KG entities, episodes, procedures, beliefs, goals,
events, tool results, wards, and resources. The relationships among these
things are present in pieces, but the runtime does not yet expose a single
bounded context graph packet for a turn.

### 5. Distillation Is Not The Same As Context Building

Distillation should create durable memory and graph material. Context building
should select the best small working set for the current turn. Today those
concepts overlap through recall, working memory, and sleep-cycle behavior.
They need clearer boundaries.

## Future State

### Capability Model

Introduce a zbot-owned Context Capability Registry. It should register and
emit four views:

1. `tools`: executable actions.
2. `resources`: read-only retrievable context.
3. `context_graph`: bounded graph expansion and context packet assembly.
4. `capability_catalog`: UI/API metadata for all capabilities.

Each capability should carry at least:

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

The existing actor hard filters stay. The registry describes and routes
capabilities, but it does not replace runtime enforcement.

### Tool Boundary

Future model-visible tools should be actions:

- `shell`
- `write_file`
- `edit_file`
- `delegate`
- `respond`
- `ingest`
- `memory_write`
- `procedure_run`
- connector capability invocation
- user-visible state mutations

Future resources should cover read-only context:

- file reads and grep results when safe to prefetch
- memory fact lookup
- belief lookup
- contradiction lookup
- graph entity lookup
- graph neighborhood lookup
- connector GET resources
- previous tool result handles
- ward metadata
- skills, agents, procedures, and docs

Some current tools can remain as compatibility wrappers during migration, but
their internals should route through the registry.

### Memory Build

Durable memory should be built from these sources:

- Explicit `memory_write` / `save_fact` requests.
- User corrections.
- Session distillation.
- Tool result distillation.
- Resource read distillation when configured.
- Ward artifact indexing.
- Human UI edits.
- Sleep-cycle consolidation.

Memory records should include:

- `id`
- `agent_id`
- `scope`
- `category`
- `subject`
- `key`
- `content`
- `confidence`
- `valid_from`
- `valid_until`
- `source_episode_ids`
- `source_tool_call_ids`
- `source_resource_uris`
- `ontology_type`
- `taxonomy_concepts`
- `embedding`
- `superseded_by`
- `contradicted_by`
- `retention_policy`

Engram should own the durable framework. Zbot should own which ontology and
taxonomy concepts apply, when facts are written, when facts are cleaned up, and
how facts enter the user-facing contract.

### Knowledge Graph Build

The graph should be built from the same evidence stream as memory:

- Conversation episodes.
- Distilled facts.
- Structured ingest payloads.
- Tool results.
- Connector resources.
- Ward artifacts.
- Procedures and skills.
- Human-curated corrections.

Graph nodes should include:

- entities
- concepts
- users
- agents
- skills
- tools
- resources
- sessions
- episodes
- tool calls
- artifacts
- procedures
- goals
- beliefs
- taxonomy concepts

Graph edges should include:

- `mentions`
- `derived_from`
- `supports`
- `contradicts`
- `supersedes`
- `related_to`
- `uses_tool`
- `produced`
- `read_resource`
- `in_ward`
- `has_taxonomy_concept`
- `instance_of`
- `broader_than`
- `narrower_than`
- `valid_during`

Edges should carry confidence, provenance, temporal validity, actor visibility,
and source pointers. The context graph should be a bounded read model over this
durable material, not a competing truth store.

### Distillation Process

Distillation should become a pipeline with explicit stages.

1. Capture raw evidence.
   Store conversation turns, tool calls, tool results, resource reads,
   artifacts, user corrections, and UI edits with stable ids.

2. Normalize evidence.
   Redact secrets, classify source type, attach agent/session/ward scope, trim
   raw payloads, and preserve handles to full raw data.

3. Extract candidates.
   Produce candidate facts, entities, relationships, beliefs, procedures,
   summaries, corrections, and taxonomy assignments.

4. Validate candidates.
   Check schema validity, relationship endpoint existence, source pointers,
   scope, actor visibility, confidence floors, and contradiction/supersession
   hints.

5. Score candidates.
   Apply source reliability, recency, repetition, user-confirmation boost,
   tool-confidence boost, contradiction penalty, and domain-specific policy.

6. Commit durable outputs.
   Write accepted memory facts, graph entities, graph relationships, beliefs,
   procedures, episodes, hierarchy nodes, and distillation run stats.

7. Consolidate.
   Sleep-cycle workers merge duplicates, abstract repeated corrections into
   schemas, synthesize beliefs, detect contradictions, decay stale knowledge,
   build hierarchy, and prune low-value material.

8. Publish observability.
   Emit run stats: candidates produced, accepted, rejected, deduped,
   superseded, contradicted, linked, unlinked, and skipped due to unsupported
   capabilities.

Distillation should not decide the active prompt by itself. It prepares durable
material. Context building chooses what to use now.

### Context Build Process

Each model call should receive a structured context packet built before prompt
rendering.

1. Intake.
   Read user message, agent config, active ward, active goals, conversation
   state, prior tool results, and actor kind.

2. Intent and route planning.
   Decide whether the request needs memory, graph, files, connector resources,
   procedures, code context, prior episodes, or no retrieval. Existing query
   gate logic should be reused and extended.

3. Candidate retrieval.
   Retrieve facts, beliefs, wiki entries, procedures, graph seeds, graph
   traversal hits, previous episodes, goals, skills, agents, connector
   resources, and recent tool-result summaries.

4. Entity linking and taxonomy expansion.
   Link mentioned entities to graph ids. Expand through durable SKOS-style
   taxonomy concepts when useful, but keep expansion bounded.

5. Context graph expansion.
   Build a small subgraph around the task using typed nodes and edges. Include
   only relevant provenance paths, not raw tables.

6. Fusion and scoring.
   Use RRF/MMR, category weights, ward boosts, recency, confidence,
   contradiction penalties, supersession filters, route hints, and actor
   visibility.

7. Budgeting.
   Allocate token budgets by lane: instructions, task state, recalled memory,
   graph context, procedures, artifacts, tool result summaries, and active
   working memory. Drop low-value items before rendering.

8. Render.
   Convert the packet into concise model text with stable handles. Raw data
   should be available through resource handles, not pasted by default.

9. Observe.
   Emit a context build trace for Observatory: candidates considered, selected,
   dropped, token estimates, source mix, and graph path.

10. Write back.
   After the model and tools run, update working memory, capture tool results,
   run micro-recall triggers, and enqueue distillation candidates.

### Context Packet Shape

A future `ContextPacket` should be structured before rendering:

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

Each `ContextAtom` should include:

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

The renderer should be deterministic. The model should see short text,
citations/handles, and a clear split between instructions, current task state,
memory, graph context, procedures, and active constraints.

### Recall Future State

Recall should become a queryable service that returns `ContextAtom`s and trace
metadata.

The service should support:

- bootstrap recall at session start
- pre-turn recall
- mid-turn recall after tool results
- micro-recall triggers
- manual recall from tools/UI
- sleep-cycle feedback

Recall should improve accuracy by:

- using explicit route decisions before retrieval
- linking entities before semantic search when possible
- expanding through graph and taxonomy only within budget
- always preserving provenance
- retaining contradiction and supersession state
- keeping corrections and user-confirmed preferences high priority
- using MMR to reduce repeated near-duplicates
- recording dropped candidates for debugging

### Micro-Recall Future State

Micro-recall should remain event-driven, but become structured.

Future triggers:

- tool error
- pre-delegation
- delegation callback
- ward entry
- new entity mention
- file write/edit completion
- connector resource read
- high-risk tool result
- contradiction signal
- user correction
- unresolved plan step

Future micro-recall output should be a mini `ContextPacketDelta`, not direct
markdown. The execution loop can then merge, score, render, and observe it the
same way as normal recall.

### Accuracy Improvements

Accuracy should improve through structure, not larger prompts.

Required improvements:

- Replace regex-only entity detection with entity linking backed by KG aliases,
  taxonomy concepts, and recent-session entities.
- Track why each context atom was selected.
- Deduplicate facts, graph hits, and summaries before rendering.
- Penalize stale current facts while preserving archival facts.
- Prefer user-confirmed corrections over inferred summaries.
- Treat contradictions as first-class selection data.
- Keep context graph expansion path-limited and confidence-aware.
- Render source handles so the model can ask for more instead of guessing.
- Log selected and dropped context so bad recall can be debugged.

## Journey Evidence From Conversation DB

The current conversation DB at `~/Documents/zbot/data/conversations.db` has
enough data to derive journey fixtures:

- `sessions`: 8 rows
- `agent_executions`: 8 rows
- `messages`: 307 rows
- `execution_logs`: 342 rows
- `artifacts`: 13 rows
- `distillation_runs`: 8 rows
- `recall_log`: 0 rows

The main end-to-end trace is session
`sess-cadd32aa-d7c1-4863-b21b-d9262bb0630f`, titled "AAPL Valuation
Analysis". It contains one root execution plus planner, builder, and writer
subexecutions:

- `exec-d00f9e0f-25cc-42bd-9cc8-2f612b6d3eb9`: root orchestration.
- `exec-5f01d5aa-3774-4914-84e5-90d9f495e32b`: `planner-agent`.
- `exec-fce94767-fa1f-4948-a1bf-b038d0e6e022`: ward setup by
  `builder-agent`.
- `exec-efc820c7-7f74-4d0d-a67f-219e63824355`: market data and fundamentals
  by `builder-agent`.
- `exec-eb16f60f-4494-47f5-b13b-dc8bb8ad45be`: valuation models by
  `builder-agent`.
- `exec-10b16a51-8bd2-41f9-8f1c-c4fa6774cae1`: final report by
  `writing-agent`.

The smaller follow-up trace is session
`sess-1bb5d404-50c3-41c4-996c-be6a8077103d`, titled "XOM Valuation
Analysis", with root plus `ward:financial-analysis`.

Tool-call counts across the DB:

| Tool | Calls | What it means |
| --- | ---: | --- |
| `shell` | 61 | Dominant path for file discovery, grep, Python execution, JSON analysis, and validation. |
| `read` | 38 | Direct file/resource reads and offloaded result reads. |
| `write_file` | 12 | Artifact/script/report creation. |
| `edit_file` | 11 | Spec and generated file correction. |
| `memory` | 6 | Recall and ctx reads. Omitted from journey diagrams because memory is omnipresent. |
| `load_skill` | 6 | Skill instructions loaded into context; several results offloaded. |
| `delegate_to_agent` | 6 | Planner/builder/writer/ward delegation. |
| `update_plan` | 5 | Ward-agent local planning. |
| `respond` | 4 | Finalization. |
| `ward` | 3 | Ward selection and routing. |
| `set_session_title` | 3 | UI title mutation. |
| `grep` | 1 | Direct structured search was barely used; agents mostly used shell grep. |

Loaded skills:

- `spec-builder`
- `plan-composer`
- `ward-designer`
- `yfinance-market-analysis`

Artifacts created in the AAPL journey include market data, fundamentals,
peer data, fetch/validation scripts, valuation result JSON, valuation summary
JSON, and the final markdown report. Distillation succeeded for every session;
the AAPL child sessions extracted between 8 and 10 facts each and between 5
and 13 entities each.

The DB does not contain an MCP-backed execution. Any MCP journey below is a
target fixture, not a claim about the observed trace.

## Journey Fixtures

Memory is deliberately not shown as a lane in these journeys. It should run as
ambient recall, context build, micro-recall, and distillation throughout every
journey.

### Journey 1: Skills + Code/File Analysis

This journey is based on the AAPL session in the conversation DB.

1. User asks for a valuation verdict.
2. Intent analysis classifies the task as `stock-valuation-analysis` and root
   selects the `financial-analysis` ward.
3. Root receives a small context packet:
   - relevant ward metadata
   - relevant skill candidates
   - current task state
   - prior artifact handles, if any
4. Root delegates planning to `planner-agent`.
5. Planner loads `spec-builder` and `plan-composer`.
6. Planner analyzes the ward file tree, existing specs, and step files.
   Current trace used shell-heavy commands such as `find`, `cat`, `grep`,
   `head`, and ad hoc Python parsing. Future state should use file-index and
   skill-resource queries instead.
7. Planner repairs the plan by adding required agent/skill metadata to step
   files.
8. Root reads the plan and delegates ward setup to `builder-agent` with
   `ward-designer`.
9. Builder reads `AGENTS.md`, ward memory-bank files, and step docs.
10. Builder writes ward doctrine files and validates the setup.
11. Root delegates market-data/fundamental collection to `builder-agent` with
   `yfinance-market-analysis`.
12. Builder creates fetch and validation scripts, executes them, validates JSON,
   and registers artifacts.
13. Root delegates valuation modeling to `builder-agent`.
14. Builder creates/runs valuation code and emits DCF, relative valuation, and
   summary artifacts.
15. Root delegates report synthesis to `writing-agent`.
16. Writer reads artifact handles, extracts only needed fields, writes final
   markdown, and validates report constraints.
17. Runtime distills the journey into facts, entities, relationships, episodes,
   artifact records, and run stats.

Future context graph improvement:

- The planner should not call shell to discover files or inspect skill
  offload files. The context packet should include a ward tree summary, spec
  handles, and loaded skill summaries.
- The builder should get typed artifact handles and schema summaries instead
  of re-reading large JSON into prompt.
- The writer should receive a compact valuation `ContextPacket` with quoted
  numbers, result handles, and provenance paths, not the full market data blob.
- Code analysis should be a read-only resource lane: file tree, grep/search,
  symbol summaries, and selected snippets with line handles.

### Journey 2: Skills + MCP + Code + Research

This is the target mixed journey that the current DB does not yet cover. It
should become an e2e fixture after the capability registry lands.

Example request: "Research how project X implements MCP tool discovery,
compare it to our gateway code, and write a migration recommendation."

1. User asks for a research-and-code comparison.
2. Intent analysis classifies the task as `research + code-analysis`.
3. Context planner selects:
   - research skills
   - code-analysis resources for the current ward/repo
   - MCP resources and tools relevant to the configured servers
   - external source handles when allowed by policy
4. Capability registry emits a filtered catalog:
   - executable actions: `read_resource`, `run_code_search`, `delegate`,
     `respond`
   - MCP actions: only specific safe MCP calls needed for the request
   - resources: MCP server manifests, local code files, prior artifacts,
     research source handles
5. The model receives a small packet:
   - task
   - relevant skills
   - code areas to inspect
   - MCP capability summary
   - source map seed
6. Research subjourney gathers authoritative sources and stores source handles.
7. Code-analysis subjourney reads bounded snippets from gateway/runtime MCP
   code, tool registry code, and connector code.
8. MCP subjourney lists server-provided resources/tools through the registry,
   not through a broad `list_mcps` tool.
9. Context graph links:
   - MCP server
   - MCP tool/resource schema
   - local adapter code
   - gateway actor policy
   - prior RFC/spec artifacts
   - cited external source
10. The model writes a recommendation from the packet and asks for extra raw
    data only through handles.
11. Distillation stores research conclusions, code relationships, MCP
    capability facts, and artifacts.

Future context graph improvement:

- MCP tool discovery becomes metadata in the capability catalog, not a prompt
  round trip.
- MCP resource reads are read-only resources unless they invoke a capability.
- Code search is a context resource with symbol/file/snippet handles.
- Research sources are resources with citation/provenance metadata.
- The final packet should fit in a bounded budget even when the repo, MCP
  server, and research corpus are large.

### Journey 3: Mixed Delegation With Runtime Intervention

This journey covers the part of zbot that the AAPL trace almost needed but did
not exercise fully: steering, waiting, and failure recovery.

1. Root delegates two independent research/code subtasks.
2. Each subagent gets a context packet filtered to its role and actor policy.
3. Root reaches a BPMN-style parallel gateway: multiple child executions are
   active and the parent cannot synthesize until the required branches finish.
4. Runtime monitors tool results.
5. `wait_agent` or its successor acts as the explicit join primitive when the
   model is orchestrating parallel branches. Sequential delegation should use
   normal callbacks/result handles and should not poll.
6. Micro-recall detects a tool error, missing file, or new entity.
7. Runtime emits a `ContextPacketDelta` with the prior fix, related entity, or
   relevant artifact handle.
8. If a subagent is still running and needs intervention, root uses a control
   action such as `steer_agent` or `kill_agent`; otherwise it waits for normal
   callbacks.
9. Root synthesizes only after all required parallel branches are joined.

Future context graph improvement:

- `wait_agent` should be a parallel-gateway join action, not a generic polling
  habit. Normal sequential delegation should return completion callbacks or
  typed handles.
- Running-agent controls should be UI/runtime controls first and model-visible
  tools only when explicitly needed.
- Micro-recall should output packet deltas, not append raw markdown to working
  memory.

## Tools To Retire, Hide, Or Split

The retirement decision should be about model visibility first. Some tools
should remain as internal actions or compatibility wrappers while they
disappear from the default prompt.

### Retire From Default Model-Visible Surface

| Tool | Recommendation | Reason |
| --- | --- | --- |
| `list_tools` | Retire as a model tool. Replace with capability catalog resources and `/api/tools`. | The model should not spend a call discovering what the runtime already filtered. |
| `list_skills` | Retire as a default model tool. Replace with skill resources in the context packet. | Skill discovery belongs in context planning; the model should load only selected skills. |
| `list_mcps` | Retire as a default model tool. Replace with MCP capability/resource catalog. | MCP discovery is metadata. The model should see filtered MCP capabilities, not raw server discovery. |
| `set_session_title` | Retire as a model tool. Make it an automatic runtime/UI side effect from intent/session summary. | It consumed calls in the trace but is not task reasoning. |
| legacy `write` / `edit` aliases | Delete or keep only as hidden compatibility aliases. | `write_file` and `edit_file` are the clearer canonical tools. |
| `todo` | Retire from default. | `update_plan` is the lightweight visible planning surface. |
| `glob` | Retire from default once file-index resources exist. | File discovery should be a resource query with scoped handles, not a model tool. |

### Split Before Retiring The Broad Tool

| Tool | Future shape | Reason |
| --- | --- | --- |
| `memory` | Split into runtime recall, read-only memory resources, `memory_write`, `belief_read`, `contradiction_read`, and exact ctx resources. | Current tool mixes durable writes, recall, beliefs, contradictions, and ctx reads. |
| `query_resource` | Split into read-only connector resources and side-effecting connector actions. | `list_resources`, `query`, and `invoke` have different security and context semantics. |
| `graph_query` | Replace default use with context graph resources; keep admin/debug graph query. | Graph context should usually be preassembled and bounded before the model call. |
| `grep` | Replace most uses with code/file search resources; keep as compatibility/debug. | The trace used shell grep far more than `grep`, showing the current search surface is not ergonomic enough. |
| `shell` | Keep for execution, but remove file discovery, grep, JSON extraction, and report validation patterns from normal prompts. | `shell` dominated the trace because structured alternatives were missing or inconvenient. |
| `ward` | Keep as a runtime routing action, but prefer intent-driven ward selection and ward resources. | The model should not manually rediscover ward context every turn. |
| `load_skill` | Keep, but change output to skill handle + bounded summary + explicit sections. | Current skill loads created large offloaded results and repeated reads. |

### Keep As Actions

| Tool | Reason |
| --- | --- |
| `read` | Still needed for explicit file reads, but known context should arrive as resources first. |
| `write_file` | Required side-effecting action. |
| `edit_file` | Required side-effecting action. |
| `delegate_to_agent` | Required orchestration primitive. |
| `wait_agent` | Required for BPMN-style parallel gateway joins when the parent must wait on multiple child executions before synthesis. Hide from sequential flows. |
| `respond` | Required finalization primitive. |
| `update_plan` | Useful compact planning state. |
| `ingest` | Required write path for structured/text graph ingestion. |
| `run_procedure` | Keep if procedures become durable first-class workflows. |
| `multimodal_analyze` | Keep hidden until multimodal input requires it. |
| `kill_agent` | Keep as runtime/UI control and expose to model only for explicit control tasks. |
| `steer_agent` / `handoff_to_agent` | Keep for live multi-agent intervention, but hide from ordinary single-agent journeys. |

### Candidate To Remove Or Redesign

| Tool | Recommendation | Reason |
| --- | --- | --- |
| `execution_graph` | Quarantine as experimental unless it becomes the internal planner/orchestrator. | It overlaps with delegation, procedures, and future context-planned journeys. |
| `python` | Do not expose as separate default if `shell` remains; consider a constrained code-runner resource/action instead. | The trace used Python through shell. A dedicated runner may be better, but two generic execution tools are redundant. |
| `web_fetch` | Hide behind research/source resources. | Research should produce source handles and citations, not raw web fetch blobs in prompt. |
| `request_input` / `show_content` | Keep UI-only unless a journey explicitly needs interactive user input or rendered content. | They are product UI actions, not general reasoning tools. |

The immediate cleanup should not delete code blindly. First, move obsolete
tools out of the default model-visible catalog. Then measure whether any
journey still needs them through explicit fallback or UI/debug paths.

## Missing Details

These details are not settled and should be resolved in the RFC/spec sequence.

1. Crate boundary.
   Decide whether the first implementation lives in `gateway-memory`,
   `gateway-context`, `agent-runtime`, or a new shared crate. Recommended
   default: start in gateway memory/execution boundaries, then extract once the
   packet and registry shapes stabilize.

2. Capability schema.
   Define the exact registry DTO and whether it lives in Rust only first or
   also under `contracts/mcp/`.

3. Context packet schema.
   Define `ContextPacket`, `ContextAtom`, `ContextGraphNode`,
   `ContextGraphEdge`, `ResourceHandle`, and `ToolResultHandle`.

4. Engram adapter ports.
   Confirm which Engram APIs cover facts, entities, relationships, beliefs,
   hierarchy, procedures, episodes, recall logs, distillation run stats, and
   embeddings. Unsupported capability behavior must fail closed.

5. Ontology and taxonomy mapping.
   Define how zbot-owned dynamic ontology types map onto durable SKOS-style
   taxonomy concepts, and how taxonomy expansion is bounded during recall.

6. Scoring formula.
   Write down the cross-source scoring formula so memory, graph, procedures,
   beliefs, and resources can be compared without accidental dominance.

7. Budget policy.
   Define per-lane token budgets and what gets dropped first under pressure.

8. Result envelope.
   Standardize tool/resource results into model-visible summary, structured
   payload, raw handle, UI payload, persistence payload, and trace metadata.

9. UI and API contract.
   Decide how Observatory, Memory, Graph, and tool monitor display capability
   catalogs, context packets, recall traces, and distillation runs.

10. Migration path.
   Decide which old model-visible tool actions remain as compatibility wrappers
   and when broad `memory`, `graph_query`, and `query_resource` actions can be
   retired or hidden from default prompts.

11. Metrics.
   Define success measures: prompt tokens per turn, tool calls per turn,
   recall hit rate, dropped-context rate, answer parity, fallback rate, latency,
   and manual correction rate.

12. Security.
   Define visibility rules for resource handles, connector data, raw tool
   result handles, credentials, and cross-agent memory.

13. Degraded mode.
   Define behavior when embeddings, Engram, KG, MCP, connectors, or the context
   graph assembler are unavailable.

14. Rig and MCP bridge.
   Resolve MCP lifecycle support on the Rig path. The capability registry
   should be engine-independent, but the execution bridge must not orphan MCP
   processes or lose cleanup.

15. Deletion and retention.
   Define how memory deletion, resource handle expiry, raw offload cleanup, and
   graph provenance removal work together.

## Implementation Sequence

1. Add read-only capability catalog.
   Populate it from the existing gateway registry, MCP manager, connector
   provider, memory service, graph service, and recall service. Wire
   `/api/tools` to it.

2. Define result envelopes.
   Standardize model-visible summary, structured content, raw handle, UI
   payload, persistence payload, and trace metadata.

3. Define `ContextAtom` and `ContextPacket`.
   Convert unified recall output into typed atoms before rendering.

4. Add context graph assembler.
   Build bounded subgraphs over memory, KG, episodes, resources, tool results,
   and active state.

5. Convert micro-recall to packet deltas.
   Keep existing triggers, but merge output through the same packet path.

6. Split read resources from action tools.
   Leave compatibility wrappers for old tools, but make default context
   assembly prefer resources and packets.

7. Strengthen distillation.
   Make raw evidence, candidate extraction, validation, scoring, durable writes,
   consolidation, and observability explicit.

8. Add UI observability.
   Show selected/dropped context, graph paths, recall source mix, capability
   health, and distillation run stats.

9. Retire broad default tool surfaces.
   Hide or split broad `memory`, `graph_query`, and `query_resource` exposure
   once parity and observability are proven.

## What Should Not Change

- Gateway/UI event contracts should remain stable during the first phase.
- Actor policy should remain hard runtime enforcement.
- Engram should provide the memory/knowledge framework, but zbot should own
  product policy, ontology, taxonomy, sleep-cycle scheduling, cleanup, and UI
  contracts.
- Raw conversation and tool logs should remain available for audit and parity
  testing.
- Model prompts should become shorter, not more elaborate.

## References

- Local tool registry: `runtime/agent-runtime/src/tools/registry.rs`
- Tool registry construction and actor filters:
  `gateway/gateway-execution/src/invoke/executor.rs`
- Memory tool: `runtime/agent-tools/src/tools/memory.rs`
- Graph query tool: `runtime/agent-tools/src/tools/graph_query.rs`
- Ingest tool: `runtime/agent-tools/src/tools/ingest.rs`
- Connector resource tool: `runtime/agent-tools/src/tools/connectors.rs`
- Unified recall: `gateway/gateway-memory/src/recall/mod.rs`
- Micro-recall: `gateway/gateway-execution/src/invoke/micro_recall.rs`
- Working memory: `gateway/gateway-execution/src/invoke/working_memory.rs`
- Context management: `runtime/agent-runtime/src/context_management.rs`
- Summarization middleware:
  `runtime/agent-runtime/src/middleware/summarization.rs`
- Memory explanation: `docs/memory-explained.md`
- Engram cutover RFC: `docs/rfc/0011-engram-memory-engine-cutover.md`
- Context graph research note:
  `docs/rfc/0014-notes/zbot-context-graph-tools-survey.md`
- PuppyGraph context graph article:
  https://www.puppygraph.com/blog/context-graph
- Anthropic context engineering:
  https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- Model Context Protocol specification:
  https://modelcontextprotocol.io/specification/2025-06-18
