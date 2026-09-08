# Specs

Active feature specs for AgentZero.

| Spec | Status | Summary |
| --- | --- | --- |
| [Rig-only Execution](rig-only-execution/spec.md) | Implementing | Complete Rig cutover for root/subagent execution, MCP and skills; removes the legacy executor and all engine-selection fallback paths. |
| [Gateway Responsibility Boundaries](gateway-responsibility-boundaries/spec.md) | Shipped | Extracts execution control and capability inspection into focused components while preserving public behavior. |
| [Session-stop Cancellation](session-stop-cancellation/spec.md) | Implementing | Cancels queued, active, and delegated request work without affecting another session. |
| [Observatory Force-Graph Polish](observatory-force-graph-polish/spec.md) | Shipped | Makes the existing D3 Observatory graph feel like a clear, premium, layered network without changing its data or interactions. |
| [Distillation Graph Governance](distillation-graph-governance/spec.md) | Shipped | Filters session-distillation graph candidates through z-Bot's built-in entity and relationship governance without changing durable fact persistence. |
| [Engram Local Graph Repair](engram-local-graph-repair/spec.md) | Shipped | Provides a dry-run-first, backup-protected utility for removing unsupported relationships and relationship-only unknown stubs from an explicit local graph database. |
| [Planner Template Handoff](planner-template-handoff/spec.md) | Shipped | Carries the selected normalized ward template into automatic graph planning and persists applicable declared planning roles. |
| [Supply-Chain Policy Refresh](supply-chain-policy-refresh/spec.md) | Shipped | Refreshes the compatible Rust dependency graph and stale advisory policy while closing merged A2A lifecycle metadata. |
| [A2A Federation and Discovery](a2a-federation-discovery/spec.md) | Shipped | Lets explicitly paired LAN/VPN zBots discover candidates and exchange remote-safe durable work through the A2A 1.0 HTTP+JSON protocol. |
| [Durable Peer Messaging](durable-peer-messaging/spec.md) | Implementing | Lets same-session agents exchange durable asynchronous messages and scoped replies over the existing local work queue while keeping a broker-neutral transport seam. |
| [Durable Generic Agent Tasks](durable-generic-agent-tasks/spec.md) | Shipped | Persists local Research invocations as strict `agent.task.v1` work before acceptance, then resumes or monitors the existing runtime through terminal state without a broker or protocol change. |
| [Durable Queue Worker Runtime](durable-queue-worker-runtime/spec.md) | Shipped | Runs a supervised local worker with exact handler dispatch, typed payload validation, lease heartbeats, bounded concurrency/timeouts/shutdown, fail-closed store errors, and no migrated producer or production handler. |
| [Durable Work Queue](durable-work-queue/spec.md) | Shipped | Adds a local SQLite work queue with bounded envelopes, fenced leases, capped retries, dead-letter state, and a broker-neutral internal transport port without adding a broker or migrating current execution flows. |
| [Unique Model Tool Inventory](unique-tool-inventory/spec.md) | Shipped | Keeps every actor's model-visible tool names unique while preserving strict schema validation and existing capability membership. |
| [Prompt Cache and Tool Footprint](prompt-cache-tool-footprint/spec.md) | Shipped | Preserves existing prompt-cache behavior while canonicalizing, measuring, and fail-closed bounding model-visible tool inventories before provider requests. |
| [Legacy WebSocket Port Retirement](legacy-websocket-port-retirement/spec.md) | Shipped | Removes the deprecated standalone listener and makes `/ws` on the HTTP port the sole client event-stream WebSocket endpoint. |
| [UI Test Type-Debt Cleanup](ui-test-type-debt/spec.md) | Shipped | Restores one TypeScript gate across UI production and test sources by repairing stale fixtures and removing the build exclusion. |
| [P4 CI and E2E Debt Cleanup](p4-tech-debt/spec.md) | Shipped | Makes UI E2E ownership executable, upgrades deprecated action runtimes, and tracks removal of the temporary React Router advisory exception. |
| [P3 CI Stabilization](p3-ci-stabilization/spec.md) | Shipped | Restores security, macOS and Windows Ward portability, and UI E2E gates without weakening their safety controls. |
| [SQLite Reindex Conformance](sqlite-reindex-conformance/spec.md) | Shipped | Makes SQLite embedding reindexing skip already-matching vec tables while preserving repair of missing or mismatched targets. |
| [SQLite Agent Isolation](sqlite-agent-isolation/spec.md) | Shipped | Makes the agent-scoped list-entities contract executable: requested-agent plus explicitly global rows, never another private agent. |
| [P0 Supply-Chain Gates](p0-supply-chain-gates/spec.md) | Shipped | Updates vulnerable Rust dependencies and restores evidence-backed dependency license checks without advisory or unlicensed bypasses. |
| [LLM Wiki Ward Foundation](llm-wiki-ward-foundation/spec.md) | Shipped | Gives every fresh archetype one Ward-named canonical page and append-only log, keeps one global catalog, replaces mandatory OKF with compact plain Markdown, and leaves optional Wiki namespaces lazy. |
| [Ward Archetype Registry and Creation](ward-archetype-registry-and-creation/spec.md) | Draft | Adds the closed local registry, explicit generic/coding creation, bounded doctrine/starters, immutable snapshots, and durable provenance. |
| [Intent Ward Archetype Selection](intent-ward-archetype-selection/spec.md) | Draft | Binds a closed create-only intent recommendation to ward creation with explicit override, deterministic generic fallback, and immutable reuse. |
| [Compact Bundled Ward Archetypes](bundled-ward-archetypes/spec.md) | Shipped | Makes all seven bundles complete but lazy, with shared concept-local planning, one canonical Ward page, and discrete date-routed journal entries. |
| [Ward Agent Doctrine Template](ward-agent-doctrine-template/spec.md) | Shipped | Moves new-Ward persona scaffolding into a bounded, user-editable template while preserving existing wards and orchestration behavior. |
| [Ward Agent Persona](ward-agent-persona/spec.md) | Shipped | Makes every ward a persistent, user-editable agent with a ward-specific identity, persona, operating contract, self-maintenance rules, and handoff. |
| [OKF Ward Tool Capabilities](okf-ward-tool-capabilities/spec.md) | Shipped | Injects one active template into root context/state and confines OKF behavior to the Ward tool plus generic agent and skill Markdown. |
| [OKF Mini-Obsidian UI](okf-mini-obsidian-ui/spec.md) | Archived | Adds validated Markdown authoring, backlinks, graph navigation, filters, and concept-local spec/plan/task views over the superseded OKF foundation. |
| [Ollama Cloud Provider](ollama-cloud-provider/spec.md) | Draft | Adds authenticated Ollama Cloud commissioning with aligned `glm-5.2:cloud` agents, a `gemma4:31b-cloud` multimodal fallback, and strict separation from Ollama Local. |
| [Autonomy Ledger](autonomy-ledger/spec.md) | Shipped | Adds durable, approved cross-session decision threads with bounded resume context, Mission Control visibility, and read-only trigger eligibility. |
| [Agent-Driven Surfaces](agent-driven-surfaces/spec.md) | Draft | Adds capability-gated, catalog-constrained agent work surfaces with channel-neutral results and server-owned actions. |
| [A2UI Component Catalog](a2ui-component-catalog/spec.md) | Shipped | Adds bounded native metrics, status, structured records, timelines, and dynamic charts to the display-only work-surface catalog. |
| [Dynamic Surface Titles](dynamic-surface-titles/spec.md) | Shipped | Replaces generic A2UI type headings with bound, static, or humanized content-aware titles. |
| [Automatic Work Surfaces](automatic-work-surfaces/spec.md) | Shipped | Lets user-facing agents automatically publish validated display-only A2UI surfaces when structure materially improves an answer. |
| [Persistent Work Surfaces](persistent-work-surfaces/spec.md) | Shipped | Adds opt-in, bounded persistence and reload restoration for validated Quick Chat and Research infographics, plus an explicit clear action. |
| [Agent Commissioning](agent-commissioning/spec.md) | Shipped | Replaces the legacy setup wizard with durable, personalized model commissioning and a portable semantic profile. |
| [Commissioning Memory Profile](commissioning-memory-profile/spec.md) | Shipped | Adds an explicit recommended Full Zbot memory choice with built-in embeddings, pinned recall/governance files, and restart-verified activation. |
| [Engram Provider Adoption](engram-provider-adoption/spec.md) | Shipped | Makes the semantic-memory adapter use Engram's provider facade and expose capability-gated services without changing agent or UI behavior. |
| [Research Terminal State Reconciliation](research-terminal-state-reconciliation/spec.md) | Shipped | Makes successful Research completion deterministically reconcile the plan surface and final response. |
| [Vault Layout Standardization](vault-layout-standardization/spec.md) | Shipped | Makes vault paths, configuration names, runtime directories, and prompt loading consistent while retiring dormant OKF material. |
| [SQLite Runtime Store Split](sqlite-runtime-store-split/spec.md) | Shipped | Separates unchanged runtime conversation persistence from quarantined legacy semantic SQLite as Engram owns the active memory layer. |
| [Quiet Instrument UI](quiet-instrument-ui/spec.md) | Shipped | Unifies every zbot UI route under a restrained, premium operational visual system while preserving all behavior and contracts. |
| [Quick Chat Terminal Response Deduplication](quick-chat-terminal-response-dedup/spec.md) | Shipped | Prevents a root terminal fallback from rendering a second Quick Chat answer. |
| [Quick Chat Recall Ranking](quick-chat-recall-ranking/spec.md) | Shipped | Preserves hybrid recall ranking and supplies query-scoped profile facts from durable memory. |
| [Graph Planning Gate](graph-planning-gate/spec.md) | Shipped | Enforces cold graph work as ward setup → planner-agent → plan-step execution. |
| [Execution Capabilities](execution-capabilities/spec.md) | Shipped | Makes skills and MCPs dynamically assigned to root and subagent executions by intent and planning, while retaining static agent mappings as a compatibility fallback. |
| [MCP ID Normalization](mcp-id-normalization/spec.md) | Shipped | Derives stable IDs for API-managed MCPs and gives the active Blender MCP its canonical `blender-mcp` ID. |
| [Attention Radar UI Rollout](attention-radar-ui-rollout/spec.md) | Archived | Uses the Mission Control Attention Radar as the successor visual contract and route-by-route rollout plan for the full z-Bot UI. |
| [Research Context Inspector](research-context-inspector/spec.md) | Implementing | Moves existing intent analysis and delegated-agent context into Research's real right inspector without adding dashboard metrics. |
| [YFinance Market Analysis Skill Consolidation](yfinance-market-analysis-skill/spec.md) | Shipped | Consolidates bundled yfinance workflows into one primary skill while keeping old `yf-*` IDs as compatibility wrappers. |
| [Tool Waste Visibility](tool-waste-visibility/spec.md) | Done | Makes blocked hooks, invalid tool arguments, planner skill drift, and tool durations visible in existing session telemetry. |
| [Mission Control Performance](mission-control-performance/spec.md) | Shipped | Makes Mission Control load bounded summary data first, then lazy-load selected-session detail as the database grows. |
| [Mission Control Attention Radar](mission-control-attention-radar/spec.md) | Shipped | Replaces Mission Control polling with a bounded live Attention Radar and focused, on-demand observability. |
| [Observatory Scale](observatory-scale/spec.md) | Shipped | Keeps Observatory summaries bounded and responsive as the Engram-backed knowledge graph grows beyond 100,000 entities and relationships. |
| [GitHub Release Installer](github-release-installer/spec.md) | Closed | Defines GitHub Release installers and artifact packaging for Linux, macOS, and Windows. |
| [Release On Main](release-on-main/spec.md) | Done for now | Automates the daily CalVer release bump and tag when changes land on `main`, while preserving the manual release script. |
| [Agent Handoff Notes](agent-handoff-notes/spec.md) | Done | Adds current-session agent discovery and one-way handoff notes over existing steering without implementing full Pattern 4 peer messaging. |
| [Runtime Context Control](runtime-context-control/spec.md) | Shipped | Consolidates live conversation compaction into runtime middleware while preserving durable memory boundaries. |
| [Rig Engine Migration](rig-engine-migration/spec.md) | Shipped | Replaces the legacy execution engine with a Rig-backed execution facade while preserving gateway/UI, config, memory, and parity contracts. |
| [Memory Hygiene](memory-hygiene/spec.md) | Closed | Superseded by Engram memory cutover, embedding-backed recall, context capability registry, and runtime context control. |
| [Durable Ward Memory](durable-ward-memory/spec.md) | Closed | Defines Layer 4 as `knowledge.db` first-level indexing over durable executable ward workspaces, with preserved ward/file/artifact route hints. |
| [Engram Memory Engine Cutover](engram-memory-engine-cutover/spec.md) | Shipped | Switches durable memory, knowledge, graph, belief, hierarchy, and recall backing to Engram through a fail-closed AgentZero adapter while preserving gateway/UI/sleep-cycle contracts. |
| [Dynamic Ontology and SKOS Taxonomy](dynamic-ontology-skos-taxonomy/spec.md) | Implementing | Adds zbot-owned dynamic ontology policy and durable SKOS-style taxonomy classification over Engram without changing gateway/UI contracts. |
| [Context Capability Registry](context-capability-registry/spec.md) | Shipped | Implements RFC-0014 with an actor-filtered capability catalog, bounded context packets, structured recall/micro-recall, and staged tool-surface cleanup. |
| [Unified Recall Default](unified-recall-default/spec.md) | Shipped | Makes a bounded model-visible recall surface use unified semantic retrieval, dynamic taxonomy expansion, and a quality gate before ontology-aware ranking. |
| [Research Submit Visibility](research-submit-visibility/spec.md) | Shipped | Keeps submitted Research requests visible across intent analysis, snapshots, and delegation. |
| [Research Live Artifact Refresh](research-live-artifact-refresh/spec.md) | Shipped | Makes newly completed Research artifacts appear in the open session without a page reload. |
| [Research Goal Deliverables](research-goal-deliverables/spec.md) | Shipped | Shows only explicitly designated goal deliverables in Research attachments while leaving working files in the ward explorer. |
| [Memory Command Deck Density](memory-command-deck-density/spec.md) | Shipped | Makes Memory's gallery-aligned scope, evidence, and curation panes remain compact and independently scrollable with large datasets. |
| [Intent Ward Execution Binding](intent-ward-execution-binding/spec.md) | Shipped | Makes intent-selected existing wards actual session and tool workspaces rather than display-only recommendations. |
| [Research Session Switching](research-session-switching/spec.md) | Shipped | Prevents stale Research state from overwriting a selected completed-session route. |
| [Goal Artifacts](goal-artifacts/spec.md) | Shipped | Makes Quick Chat show only explicitly designated, safely served goal deliverables. |
| [Session Plan Monitoring](session-plan-monitoring/spec.md) | Shipped | Separates live session plans from durable decision threads in Research and Mission Control. |
| [Connector Resource Invoke Split](connector-resource-invoke-split/spec.md) | Shipped | Splits broad connector resource querying from side-effecting connector invocation in the model-visible tool surface. |
| [Conversation Store Revamp](conversation-store-revamp/spec.md) | Complete | Splits conversations.db into zbot-conversation (messages/checkpoints/thread_summaries) + zbot-trace (slim execution_logs + streamed .jsonl.zst + DuckDB analytics); promotes a real versioned checkpoint (replay→O(1)); clean cutover + dead-code deletion. |
| [Conversation Compatibility Retirement](conversation-compat-retirement/spec.md) | Complete | Retires the legacy `ConversationRepository`/`ConversationStore` compatibility layer now that Engram owns semantic memory and `zbot-conversation` owns transcripts. |
| [Embedding-Backed Memory Recall](embedding-backed-memory-recall/spec.md) | Shipped | Makes normal hybrid memory recall use configured FastEmbed/Ollama query embeddings, fail closed on unsafe lexical fallback, and fuse sparse/dense evidence with provenance. |
| [Vault Ward Browser](vault-ward-browser/spec.md) | Shipped | Adds a read-only Vault tab for browsing ward filesystem trees and previewing common files through bounded local-only APIs. |
| [Ward Vault In Research](ward-vault-in-research/spec.md) | Done | Embeds a read-only ward-scoped Vault explorer/search pane inside Research after a session has an active ward. |
| [Simplified Provider Model Configuration](simplified-provider-model-configuration/spec.md) | Implemented | Replaces broad model metadata maintenance with 200k input / 32k output defaults plus agent and Advanced overrides. |
| [MCP OAuth](mcp-oauth/spec.md) | Done | Adds OAuth metadata, authorization flow endpoints, token storage, and runtime bearer injection for protected remote MCP servers. |
| [Subagent Capability Policy](subagent-role-gating/spec.md) | Done | Enforces root, executor, reviewer, and ward-agent tool capabilities with an explicit reviewer-agent identity. |
| [Builder Delegation Hygiene](builder-delegation-hygiene/spec.md) | Done | Adds delegation modes so builder-agent can distinguish direct artifacts, ward hygiene, ward-backed builds, and step execution. |
| [Agent Tool Surface Cleanup](agent-tool-surface-cleanup/spec.md) | Shipped | Deletes unreachable model-tool implementations, unused factory registry paths, and stale settings/UI toggles so the tool surface matches the live gateway executor. |
| [Gateway Execution Sleep Shim Cleanup](gateway-execution-sleep-shim-cleanup/spec.md) | Shipped | Removes gateway-execution compatibility shims for sleep maintenance operations now owned by gateway-memory, leaving only execution-specific handoff writing. |
