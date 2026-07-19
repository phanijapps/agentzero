# Zbot Context Graph Tool Model Survey
> Discipline: applied (practitioner-pattern survey)

## Scope

This survey looks at how zbot should reshape its tool surface around memory,
knowledge, and context graph capabilities so the runtime can spend fewer tokens
while preserving the gateway, UI, actor policy, and Engram-backed memory
direction already in flight.

The core question is not "which graph database should zbot use?" It is "what
capability contract should the model see so executable actions, retrievable
context, durable memory, and graph-derived working sets stop competing for the
same prompt budget?"

## Findings

- Finding: A context graph is best treated as a context assembly layer over
  entities, relationships, provenance, temporal state, and decision traces, not
  as a replacement for memory or the knowledge graph. [moderate]
  Downgrade: vendor-blogged; indirectness.
  Evidence: PuppyGraph defines a context graph as entities plus relationships
  plus operational context such as timestamps, confidence, source, governance,
  and decision traces. Graphiti/Zep uses temporal context graphs as dynamic
  agent memory, while Microsoft GraphRAG uses graph structure to assemble
  query-focused context from a corpus. These converge on graph-shaped retrieval
  and assembly, but "context graph" itself is still mostly vendor and emerging
  terminology.

- Finding: The capability surface should distinguish executable tools,
  read-only resources, reusable prompts/workflows, and model-ready context
  envelopes. [high]
  Evidence: The MCP specification separates tools, resources, and prompts; it
  treats tools as model-invoked functions and resources as application-managed
  context. Anthropic's agent guidance also frames agents as augmented LLMs using
  retrieval, tools, and memory, with clear tool interfaces and tested tool-use
  behavior. This maps directly onto zbot's need to expose context without
  turning every read into a model-visible action.

- Finding: Token savings should come from bounded context envelopes,
  progressive disclosure, resource links, persisted prompt-safe summaries, and
  raw-result offload rather than from simply adding more tools or increasing the
  context window. [high]
  Evidence: Anthropic's context-engineering guidance emphasizes maintaining an
  optimal finite token set through compaction, tool-result clearing, structured
  note-taking, subagent summaries, and layered context loading. zbot already has
  local precedent in runtime tool-result offload, `context_management`, and
  RFC-0009's prompt-safe result persistence direction.

- Finding: zbot already has the ingredients for memory, knowledge, and context
  graph behavior, but they are fragmented across unrelated capability paths.
  [high]
  Evidence: `memory` is a large multi-action tool for key-value memory, facts,
  recall, beliefs, and contradictions. `graph_query` separately exposes graph
  search, neighbors, and context. `query_resource` exposes connector resources.
  MCP tools have a separate manager. `gateway-memory` recall already combines
  memory, knowledge graph, episodes, wiki, procedures, beliefs, MMR, and trace
  data. `micro_recall` already injects targeted memory/KG lookups after certain
  execution events. The HTTP `/api/tools` endpoint is currently a placeholder,
  so there is no central public inventory of capabilities.

- Finding: The recommended direction is a zbot-owned Context Capability
  Registry that emits four views from one policy-controlled source: executable
  tools, readable resources, context graph queries, and UI/API capability
  metadata. [moderate]
  Downgrade: indirectness.
  Evidence: MCP provides the external shape for tools/resources/prompts, while
  zbot's existing runtime has hard actor filters, hidden `ToolContext`, schema
  hardening, and tool-result offload. The synthesis is local architecture work:
  keep those enforcement points, but stop duplicating capability discovery
  across first-party tools, MCP, connectors, memory, graph query, and UI.

- Finding: Context graph nodes should include sessions, executions, tool calls,
  tool results, memory facts, KG entities, resources, artifacts, wards,
  procedures, skills, agents, and connector objects; edges should carry
  provenance, visibility, actor scope, temporal validity, confidence, and
  source pointers. [moderate]
  Downgrade: indirectness.
  Evidence: PuppyGraph and Graphiti prior art both stress temporal and
  provenance-bearing edges. zbot already logs execution events, stores recall
  traces, carries hidden runtime context, and has separate resource and graph
  APIs. The exact schema is zbot-specific, but the modeling pressure is common:
  the model needs a small traceable subgraph, not every raw record.

- Finding: The existing `memory` mega-tool should be wrapped or split over time
  into smaller ACI-friendly capabilities with explicit output schemas and
  context/resource links. [moderate]
  Downgrade: indirectness.
  Evidence: Anthropic's tool guidance recommends clear tool definitions,
  examples, edge cases, and boundaries, and MCP supports structured output
  schemas. zbot's current `memory` tool mixes durable writes, reads, semantic
  recall, structured facts, beliefs, and contradiction checks behind one action
  enum, which makes model behavior and permissions harder to reason about.

- Finding: Actor capability policy and hidden runtime context must stay as hard
  runtime enforcement, not move into prompts, context graph metadata, or tool
  descriptions. [high]
  Evidence: zbot already hard-filters capabilities by actor kind in gateway
  execution, passes privileged state through hidden `ToolContext`, and hardens
  schemas before model exposure. MCP security guidance also requires validation,
  access controls, user confirmation for sensitive operations, rate limiting,
  sanitization, timeouts, and audit logging.

- Finding: Rig does not remove the need for this capability layer; it makes the
  layer more important because zbot's existing tools are adapted into Rig's
  tool model and MCP still has lifecycle gaps on the Rig path. [high]
  Evidence: zbot's Rig adapter wraps existing tools and preserves hidden
  runtime context. Gateway execution currently falls back from Rig when MCP
  servers are configured because MCP lifecycle bridging is not complete. A
  capability registry can be the stable boundary whether the execution engine
  is legacy, Rig-backed, or later replaced again.

## Local Prior Art

- `runtime/agent-tools/src/tools/memory.rs`: one broad memory tool combining
  key-value memory, structured facts, recall, beliefs, and contradictions.
- `runtime/agent-tools/src/tools/graph_query.rs`: graph search, neighbor, and
  context retrieval as a separate tool.
- `gateway/gateway-execution/src/invoke/micro_recall.rs`: event-triggered
  micro-recall that already behaves like just-in-time context graph expansion.
- `gateway/gateway-memory/src/recall/mod.rs`: hybrid recall over memory, KG,
  episodes, wiki, procedures, beliefs, query gates, MMR, and recall traces.
- `runtime/agent-runtime/src/context_management.rs`: tool-result compaction,
  offload, and prompt-safe context handling.
- `runtime/agent-runtime/src/tools/registry.rs`: in-memory first-party tool
  registry without durable metadata, risk classification, resource views, or UI
  inventory semantics.
- `runtime/agent-runtime/src/mcp/manager.rs`: MCP manager is separate from
  first-party registry and connector resource discovery.
- `gateway/src/http/tools.rs`: placeholder `/api/tools` implementation, which
  confirms zbot lacks a complete public capability inventory.
- `docs/rfc/0009-agent-runtime-budget-and-retry-governance.md`: prior RFC for
  prompt-safe result persistence and token-budgeted continuation.
- `docs/rfc/0011-engram-memory-engine-cutover.md`: prior RFC direction to keep
  zbot contracts stable while moving durable memory/knowledge behind Engram.
- `docs/specs/dynamic-ontology-skos-taxonomy/spec.md`: current plan for
  zbot-owned dynamic ontology plus durable SKOS-style taxonomy.

## External Prior Art

- PuppyGraph, "Context Graph: The Missing Layer for AI Agents":
  https://www.puppygraph.com/blog/context-graph
- Anthropic, "Effective context engineering for AI agents":
  https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- Anthropic, "Building effective agents":
  https://www.anthropic.com/engineering/building-effective-agents
- Model Context Protocol 2025-06-18 specification:
  https://modelcontextprotocol.io/specification/2025-06-18
- MCP server tools specification:
  https://modelcontextprotocol.io/specification/2025-06-18/server/tools
- MCP server resources specification:
  https://modelcontextprotocol.io/specification/2025-06-18/server/resources
- Microsoft Research, "From Local to Global: A Graph RAG Approach to
  Query-Focused Summarization":
  https://www.microsoft.com/en-us/research/publication/from-local-to-global-a-graph-rag-approach-to-query-focused-summarization/
- Microsoft GraphRAG documentation:
  https://microsoft.github.io/graphrag/
- Graphiti/Zep overview:
  https://help.getzep.com/graphiti/getting-started/overview
- Graphiti arXiv paper:
  https://arxiv.org/html/2501.13956v1

## Implications For The RFC

The RFC should propose a capability model, not a new graph database first. The
durable facts should remain in Engram-backed memory/knowledge stores and zbot
owned ontology/taxonomy policy. The context graph should be a derived,
bounded, traceable read model that assembles model-ready context and resource
pointers from those stores plus execution logs and connector metadata.

The tool model should move toward:

1. `tools`: executable, side-effecting or computation capabilities, with actor
   gates, schemas, audit logs, and hidden runtime context.
2. `resources`: read-only context providers with URIs, versions, scopes, and
   optional structured summaries.
3. `context_graph`: bounded graph expansion and compression over memory,
   knowledge, session, artifact, and resource nodes.
4. `capability_catalog`: UI/API-visible inventory with risk class, actor
   availability, input/output schemas, examples, and current health.

The first implementation should be additive and contract-preserving:

1. Add the registry/catalog boundary.
2. Register existing memory, graph, connector, MCP, and first-party tools into
   it.
3. Add context envelopes that return small model-visible summaries plus links
   or handles to raw data.
4. Replace prompt-level discovery and mega-tool selection gradually.
5. Measure token spend, tool calls, fallback rate, and answer parity on current
   conversation fixtures before retiring old surfaces.

## Known Unknowns

- Whether Engram now exposes enough graph/query ports to support context graph
  assembly without zbot adding a separate read-model table.
- Whether the first crate should be `gateway-context`, a runtime crate, or an
  adapter inside `gateway-memory` until the API stabilizes.
- Which UI panes need capability metadata first: Observatory, Memory, Graph,
  tool monitor, or all of them.
- Which token benchmark should be canonical: live sessions, current
  conversation database fixtures, or synthetic e2e conversations.
- RFC numbering was ambiguous because `docs/CONVENTIONS.md` references an
  RFC-0013 credential-broker contract that does not exist in `docs/rfc/`; this
  proposal moved to RFC-0014 to avoid reusing that implied number.
