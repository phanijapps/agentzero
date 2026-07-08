# Specs

Active feature specs for AgentZero.

| Spec | Status | Summary |
| --- | --- | --- |
| [YFinance Market Analysis Skill Consolidation](yfinance-market-analysis-skill/spec.md) | Shipped | Consolidates bundled yfinance workflows into one primary skill while keeping old `yf-*` IDs as compatibility wrappers. |
| [Tool Waste Visibility](tool-waste-visibility/spec.md) | Done | Makes blocked hooks, invalid tool arguments, planner skill drift, and tool durations visible in existing session telemetry. |
| [Mission Control Performance](mission-control-performance/spec.md) | Shipped | Makes Mission Control load bounded summary data first, then lazy-load selected-session detail as the database grows. |
| [GitHub Release Installer](github-release-installer/spec.md) | Closed | Defines GitHub Release installers and artifact packaging for Linux, macOS, and Windows. |
| [Release On Main](release-on-main/spec.md) | Done for now | Automates the daily CalVer release bump and tag when changes land on `main`, while preserving the manual release script. |
| [Agent Handoff Notes](agent-handoff-notes/spec.md) | Done | Adds current-session agent discovery and one-way handoff notes over existing steering without implementing full Pattern 4 peer messaging. |
| [Runtime Context Control](runtime-context-control/spec.md) | Draft | Consolidates live conversation compaction into runtime middleware while preserving `knowledge.db` durable memory. |
| [Rig Engine Migration](rig-engine-migration/spec.md) | Shipped | Replaces the legacy execution engine with a Rig-backed execution facade while preserving gateway/UI, config, memory, and parity contracts. |
| [Memory Hygiene](memory-hygiene/spec.md) | Draft | Adds durable-memory guards for recall embedding input, handoff persistence, KG relationship integrity, and hygiene observability. |
| [Durable Ward Memory](durable-ward-memory/spec.md) | Closed | Defines Layer 4 as `knowledge.db` first-level indexing over durable executable ward workspaces, with preserved ward/file/artifact route hints. |
| [Engram Memory Engine Cutover](engram-memory-engine-cutover/spec.md) | Shipped | Switches durable memory, knowledge, graph, belief, hierarchy, and recall backing to Engram through a fail-closed AgentZero adapter while preserving gateway/UI/sleep-cycle contracts. |
| [Dynamic Ontology and SKOS Taxonomy](dynamic-ontology-skos-taxonomy/spec.md) | Draft | Adds zbot-owned dynamic ontology policy and durable SKOS-style taxonomy classification over Engram without changing gateway/UI contracts. |
| [Context Capability Registry](context-capability-registry/spec.md) | Implementing | Implements RFC-0014 with an actor-filtered capability catalog, bounded context packets, structured recall/micro-recall, and staged tool-surface cleanup. |
| [Conversation Store Revamp](conversation-store-revamp/spec.md) | Complete | Splits conversations.db into zbot-conversation (messages/checkpoints/thread_summaries) + zbot-trace (slim execution_logs + streamed .jsonl.zst + DuckDB analytics); promotes a real versioned checkpoint (replay→O(1)); clean cutover + dead-code deletion. |
| [Conversation Compatibility Retirement](conversation-compat-retirement/spec.md) | Complete | Retires the legacy `ConversationRepository`/`ConversationStore` compatibility layer now that Engram owns semantic memory and `zbot-conversation` owns transcripts. |
| [Embedding-Backed Memory Recall](embedding-backed-memory-recall/spec.md) | Shipped | Makes normal hybrid memory recall use configured FastEmbed/Ollama query embeddings, fail closed on unsafe lexical fallback, and fuse sparse/dense evidence with provenance. |
| [Vault Ward Browser](vault-ward-browser/spec.md) | Shipped | Adds a read-only Vault tab for browsing ward filesystem trees and previewing common files through bounded local-only APIs. |
| [Ward Vault In Research](ward-vault-in-research/spec.md) | Done | Embeds a read-only ward-scoped Vault explorer/search pane inside Research after a session has an active ward. |
| [Simplified Provider Model Configuration](simplified-provider-model-configuration/spec.md) | Implemented | Replaces broad model metadata maintenance with 200k input / 32k output defaults plus agent and Advanced overrides. |
| [MCP OAuth](mcp-oauth/spec.md) | Done | Adds OAuth metadata, authorization flow endpoints, token storage, and runtime bearer injection for protected remote MCP servers. |
| [Subagent Capability Policy](subagent-role-gating/spec.md) | Done | Enforces root, executor, reviewer, and ward-agent tool capabilities with an explicit reviewer-agent identity. |
| [Builder Delegation Hygiene](builder-delegation-hygiene/spec.md) | Done | Adds delegation modes so builder-agent can distinguish direct artifacts, ward hygiene, ward-backed builds, and step execution. |
