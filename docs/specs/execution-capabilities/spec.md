# Spec: Execution Capabilities

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make skills and MCPs execution-scoped capabilities rather than permanent agent
configuration. Intent analysis provides a recommended `agent -> {skills, mcps}`
mapping for simple root work and as guidance for graph work. The graph planner
can assign any currently configured, enabled, runtime-ready skill or MCP to a
plan step, taking intent guidance into account. A delegated child receives its
resolved assignment before its first model turn: assigned MCP tools are started
and visible in its tool-discovery surface, and assigned skills are available to
the existing skill-loading flow as explicit recommendations, not eagerly loaded
content. Existing agent `mcps` settings continue as a no-migration compatibility
fallback only when no dynamic assignment was made. Existing `AgentConfig.skills`
does not provide equivalent runtime loading today and is not promoted as a
compatibility behavior by this feature.

## Boundaries

### Always do

- Represent an assignment as one shared, serializable shape containing an
  agent ID, skill IDs, and canonical MCP IDs; preserve the distinction between
  an absent assignment and an explicit empty assignment.
- Build intent and planner catalogs from the live skill and MCP services.
  Intent may use a relevant subset; planner receives a safe overview and a
  bounded, planner-only capability lookup over the complete live catalog, so it
  may choose any item. Intent recommendations are guidance, not a ceiling.
- Validate every capability-assignment target against the live agent catalog,
  except `root` and an existing safe `ward:<name>` virtual target. Reject an
  invalid target assignment rather than granting capabilities to an
  auto-created specialist.
- Validate every selected MCP against current enabled and OAuth-ready runtime
  configuration immediately before it is mounted; drop unavailable entries
  safely and record a non-secret diagnostic.
- Preserve exact canonical-ID precedence through startup. A dynamic ID must
  never also select a different server whose display name happens to equal
  that ID; display-name aliases remain legacy-static compatibility only.
- Add assigned MCPs to the delegated executor before its first model request,
  so their tools participate in normal tool discovery. Keep the planner's
  catalog descriptive only: planning must not start MCP servers.
- Transfer and register the complete planner catalog only for a host-owned
  planning transition. A `step_executor` receives neither capability lookup
  nor agent delegation, including when its execution target is `ward:<name>`.
- Require planner step briefings to preserve exact `## Skills` and `## MCPs`
  canonical-ID fields (or explicit `none`) through session plan state so root
  can delegate the selected assignment without reconstructing it from prose.
- Record proposed and resolved assignments using validated canonical IDs only.
  For unresolved model requests, log a count and a closed reason code
  (`unknown_id`, `disabled`, `oauth_unavailable`, `deleted`, or
  `startup_failed`) rather than the raw request or runtime error text. Never
  log URLs, commands, headers, credentials, or OAuth material.
- Treat a selected MCP that fails process or transport startup as unavailable:
  register none of its tools, perform no automatic retry, log only
  `startup_failed`, and allow the child to continue without that MCP.
- Preserve the current `load_skill` behavior: assigned skills are passed as
  explicit child recommendations and are not pre-loaded. Retain existing static
  MCP mappings only when the invocation has no dynamic assignment at all.

### Ask first

- Removing the legacy static fallback, deleting `AgentConfig.skills` or
  `AgentConfig.mcps`, or writing a config-file migration.
- Adding a user-facing capability-mapping UI, a new public REST/WebSocket
  payload, a database schema, a new dependency, or a new top-level module.
- Allowing an MCP that is disabled or OAuth-disconnected at execution time to
  start because it was selected earlier.

### Never do

- Never mount every configured MCP on root, planner, or each child merely to
  make it discoverable.
- Never treat the LLM's capability IDs as authoritative; only current service
  catalog entries may be used, and runtime MCP auth is rechecked at startup.
- Never expose MCP connection details or secret configuration through prompts,
  logs, intent snapshots, delegated context, or tool results.
- Never serialize raw model-requested unknown IDs or MCP startup error strings
  into execution logs or prompt state.
- Never silently apply a legacy agent mapping after an explicit dynamic empty
  assignment. A present assignment owns both dimensions, so an empty skills or
  MCP array intentionally selects none of that capability type.

## Testing Strategy

- Capability normalization and fallback semantics: **TDD**. Unit tests cover
  canonical IDs, deduplication, unavailable servers, explicit-empty versus
  absent assignments, and safe log payloads.
- Intent and planner routing: **TDD**. Typed-output and planner-context tests
  prove that intent supplies guidance while a graph plan may select any live
  catalog capability.
- Delegated discovery: **TDD**, exercised through gateway-execution
  integration tests. A child receives only its mapped MCP tool before its
  first model turn; the planner receives no live MCP tools.
- Quick Chat root routing and legacy compatibility: **goal-based check**.
  Focused tests prove the bounded chat selector can attach root capabilities
  without creating a ward, planner, or delegation, and unchanged agents still
  work when no dynamic assignment exists.
- Regression and real-server smoke: **manual QA**. Use the configured Blender
  MCP in a graph request and inspect the execution trace for a Blender MCP
  tool call rather than an unprompted shell fallback.

## Acceptance Criteria

- [x] **AC-root-simple:** Given a simple Research or Quick Chat request that needs a configured
  MCP, when capability selection maps it to `root`, root starts only that
  validated MCP before answering and no planner or child agent is created.
- [x] **AC-graph-catalog:** Given a graph request, intent output contains an agent capability map as
  guidance and both cold and graduated-existing-ward graph routes forward that
  guidance to their planning executor. That executor receives a safe catalog
  overview and can page or search every current skill and MCP through a
  planner-only lookup tool without starting an MCP server.
- [x] **AC-delegated-discovery:** Given a planner step assigning `builder-agent` the Blender MCP, when the
  child starts, Blender MCP tools are present in that child's discoverable tool
  surface before the first LLM request; no unrelated configured MCP is present.
  The planner's exact `## Skills` and `## MCPs` briefing fields survive session
  plan storage and become the delegation assignment.
- [x] **AC-planner-nonceiling:** Given a planner step assigning a capability that intent did not
  recommend or semantic retrieval did not return, when that capability is
  currently available and its target agent is valid, the assignment is accepted
  and executed; intent does not restrict planner choice.
- [x] **AC-catalog-complete:** Given the complete capability catalog exceeds planner prompt budget, when
  planner looks up a valid canonical ID or searches its safe metadata, it can
  discover and assign that capability without MCP startup or arbitrary catalog
  truncation.
- [x] **AC-target-validation:** Given an invalid capability-assignment target, the mapping is rejected
  before executor construction and cannot attach an MCP to an auto-created
  specialist. `root` and an existing safe `ward:<name>` are allowed virtual
  targets.
- [x] **AC-step-least-privilege:** Given a `ward:<name>` step executor, it receives only its assigned MCP
  tools and cannot look up the complete planner catalog or delegate a
  grandchild. A ward-backed planning executor retains those planning powers.
- [x] **AC-fallback-semantics:** Given no dynamic assignment, existing agents retain their current static
  MCP behavior without a config migration. Given an explicit empty or partial
  dynamic assignment, static MCPs are not applied; assigned skills are injected
  only as `load_skill` recommendations and are not eagerly loaded. An exact
  dynamic MCP ID wins over another server's colliding display-name alias.
- [x] **AC-runtime-revalidation:** Given a selected MCP becomes disabled or OAuth-disconnected before child
  startup, it is not mounted, execution continues with a safe diagnostic, and
  no credentials or connection details appear in prompt state or logs.
- [x] **AC-startup-failure:** Given a runtime-ready MCP fails process or transport startup, none of its
  tools are registered, the child continues without automatic retry, and logs
  only canonical safe assignment data plus `startup_failed`.
- [x] **AC-lookup-bounds:** Capability lookup returns only whitelisted, bounded metadata: at most 25
  results per page, a query of at most 256 characters, and descriptions capped
  at 512 characters. Paging remains bounded by the existing planner turn/tool
  budget and exhaustion produces no unbounded lookup loop.
- [x] **AC-audit-logging:** Execution logs expose intent guidance, the planner's requested mapping,
  and the effective child assignment with an origin (`intent`, `planner`, or
  `legacy_fallback`); no UI or public protocol change is required.
- [x] **AC-verification:** Focused Rust tests, formatting, relevant workspace checks, and the
  required gateway full-mode E2E pass without altering unrelated Observatory
  worktree changes.

## Assumptions

- Technical: current agent config has static `skills` and `mcps`, intent
  analysis returns separate skill/agent lists, and delegation carries skills
  but not MCPs (source: `gateway/gateway-services/src/agents.rs`,
  `gateway/gateway-execution/src/middleware/intent_analysis.rs`,
  `runtime/agent-runtime/src/tools/delegate.rs`).
- Technical: the executor starts MCP servers from the agent's `mcps` list, and
  delegated skills are currently injected as instruction hints rather than
  pre-loaded executor state; graph nodes already assign an agent and skills
  (source: `gateway/gateway-execution/src/invoke/executor.rs`,
  `gateway/gateway-execution/src/delegation/spawn.rs`,
  `gateway/gateway-execution/src/middleware/intent_analysis.rs`).
- Technical: Quick Chat skips full intent analysis, planning, delegation, and
  ward transitions, so root capability selection needs a bounded chat-specific
  path (source: `gateway/gateway-execution/src/config.rs`,
  `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`).
- Product: planner may assign any current skill or MCP while using intent
  mapping as guidance, not an allowlist (source: user confirmation 2026-07-18).
- Product: static agent-to-capability configuration is deprecated but remains
  as a no-migration compatibility fallback for now (source: user confirmation
  2026-07-18).
- Product: assignment visibility is limited to execution and logging; no new
  UI or public protocol surface is requested (source: user confirmation
  2026-07-18).
- Process: a feature contract and implementation plan belong in one
  `docs/specs/<feature>/` directory (source: `docs/CONVENTIONS.md §4`).
