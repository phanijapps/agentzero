# Spec: Rig-only execution

- **Status:** Implementing
- **Owner:** phanijapps
- **Plan:** [plan.md](plan.md)
- **Constrained by:** runtime/AGENTS.md; runtime/agent-runtime/AGENTS.md; gateway/gateway-execution/AGENTS.md; gateway/gateway-execution/src/runner/AGENTS.md; [Runtime Context Control](../runtime-context-control/spec.md); [MCP OAuth](../mcp-oauth/spec.md); [Subagent Role Gating](../subagent-role-gating/spec.md); [Agent Handoff Notes](../agent-handoff-notes/spec.md); [Builder Delegation Hygiene](../builder-delegation-hygiene/spec.md); [Provider Configuration](../simplified-provider-model-configuration/spec.md)
- **Contract:** none (existing Rust execution/event types and HTTP/WebSocket payloads remain compatible)
- **Shape:** integration

## Objective

Every local z-Bot root agent, delegated subagent, continuation, and recovered execution runs through Rig. Configured MCP tools, built-in tools, and skill-driven actions use the same Rig tool dispatch under AgentZero authorization. No legacy executor, alternative execution loop, engine-selection environment flag, or fallback is shipped. Users retain working provider configuration, streaming UI, session persistence, controls, delegation, skills, MCP, context management, and restart recovery.

## Boundaries

### Always do

- Preserve current external configurations, persisted identifiers, conversation/checkpoint compatibility, StreamEvent → GatewayEvent → ServerMessage semantics, and terminal respond-argument persistence.
- Keep direct Rig imports in the runtime adapter. Rig owns model/tool iteration; AgentZero owns durable orchestration policy, authorization, session lifecycle, storage, and protocol delivery.
- Cover every local execution entry point, including WebSocket/HTTP, CLI/cron/connector callers, durable Research tasks, A2A ingress, delegated modes, and restart recovery. Outbound remote A2A peers retain their own engines; this process never falls back locally.
- Port required behavior before removing its implementation. Record structural retirement separately from lines moved into new modules.

### Delegated branch decisions

- The user delegates cutover implementation decisions on this branch, including the Rig/rmcp dependency selection, bounded baseline repairs and reviewed plan adjustments. No repeated approval pause is required for these decisions.
- Preserve public schemas/APIs, security controls and provider/MCP/skill support; branch authority is not permission to silently reduce parity or deploy externally.
### Ask first

- External publication/deployment, or product changes beyond this cutover (broad session controls, retry/delegation policy, or memory/distillation scheduling). Branch implementation/dependency/plan choices above are already delegated and require no further approval.

### Never do

- Ship AgentExecutor under another name, retain a test-feature legacy engine, or reimplement a second model/tool loop alongside Rig.
- Route unsupported MCP, skill, provider, or recovery configurations to a legacy executor or silently omit requested capabilities.
- Construct the legacy executor as a temporary configuration/tool container on the final Rig path.
- Pass AppState/ExecutionRunner as a service locator, create a second handle/delegation registry, add a new top-level crate, or change storage backends as part of this migration.
- Delete parity assertions merely because they expose a Rig gap; retain behavior tests on the sole engine.

## Testing Strategy

- **TDD:** adapter event mapping, tool authorization/context, cancellation and budget bounds, MCP lifecycle, skill state, context controls, and typed error paths have explicit observable contracts.
- **Integration:** production factory → real Rig engine → scripted local provider/MCP transports → actual temporary conversation/state stores. An engine-name assertion alone is insufficient: tests observe model requests, tool side effects, events, persisted results, and cleanup.
- **Goal-based:** full workspace builds/tests, static retired-symbol audit, dependency inspection, and runtime-only Rig routing across default/all-feature targets establish absence of legacy code. No stubs are authored during this planning turn.
- **End-to-end:** real daemon + mock provider + real UI Mode Full scenarios cover root, skill/MCP, delegation/continuation, cancellation and restart. Isolated CLI/cron/connector/A2A ingress tests prove the remaining adapters reach the same factory. No paid external calls or user vault mutations are required for baseline verification.

## Acceptance Criteria

- [ ] AC1: All local execution paths instantiate only Rig without AgentExecutor construction. Default, daemon/watch, release, test, example, and supported feature builds contain no live legacy engine. Setting ZBOT_ENGINE to rig, legacy, or another value cannot change execution selection; the selector and documented switch are absent.
- [ ] AC2: MCP tools on all currently supported configured transports (stdio, SSE, HTTP, and the distinct streamable-http variant) execute via Rig with unchanged tool naming/schema/result semantics and per-session hidden identity. Startup failure, auth failure, cancellation, stream error, and shutdown release only the owning session's resources; no orphan stdio processes remain after a bounded shutdown (at most 5 seconds after cancellation/shutdown is initiated).
- [ ] AC3: Skills retain configured availability, load_skill/section semantics, bounded packets, loaded-state tracking, and compaction/recovery behavior. Missing or forbidden skills fail explicitly. A skill cannot add tools, widen credentials, or bypass tool guards.
- [ ] AC4: Token/reasoning, tool start/result/error, usage/context state, respond/delegate and terminal events preserve their ordering and identifiers. Exactly one terminal outcome is published per applicable execution completion; a delegation yield is not reported as final completion. A respond-tool-only answer is persisted before completion and survives reload on initial and continuation paths.
- [ ] AC5: Stop/cancel interrupts a stalled model stream within 1 second in deterministic integration fixtures; no later tool starts after cancellation is observed. Pause/resume, iteration extension and configured iteration/token limits retain their existing effective behavior. MCP cancellation additionally meets AC2. No policy dependency error permits tool execution.
- [ ] AC6: Context budgeting, summarization/context editing, plan/skill preservation, result offload, mid-session recall, steering, and context checkpoints operate on the Rig path. Before each provider request the configured input-budget policy is applied; inability to fit the request fails explicitly. Steering and recall isolation hold across simultaneous sessions.
- [ ] AC7: Root → child → callback → continuation runs entirely through Rig, preserving DirectArtifact, WardHygiene, WardBackedBuild and StepExecutor behavior; parallel limits, wait_agent outcomes, nested delegation and cancellation isolation hold. Callback persistence precedes parent continuation, and crashes/stops do not resume canceled parents.
- [ ] AC8: Persisted root/subagent recovery preserves execution/session IDs, checkpoint/history content, scoped tool context, peer targets, and terminal-answer reconciliation. Recovery does not require serialized legacy executor state or duplicate completed tool side effects.
- [ ] AC9: Actor/tool policy and hidden execution/session/ward identity are checked before side effects, including on direct tool-name requests and MCP dispatch. Delegation does not broaden the caller's authorized scope. Missing policy/auth/config fails closed. Secret canaries are absent from model messages, model-visible tool schemas/args, HTTP/WS errors, stdout/stderr and persisted logs; retrieved/tool/skill reference content cannot overwrite authoritative instructions or hidden identity.
- [ ] AC10: One engine-construction path serves initial, continuation, recovery, and child execution with explicit dependencies. Core orchestration does not duplicate stream persistence/finalization implementations; mode-specific completion policies remain explicit. Late service wiring cannot leave pre-captured invokers with stale stores/configuration.
- [ ] AC11: AgentExecutor implementation/factory/export, legacy-only configuration and hook plumbing, selector/fallback branches, ZBOT_ENGINE launch wiring, obsolete A/B tests/examples and legacy-only dependencies are removed. Shared errors/hooks/DTOs survive only in neutral owning modules with proven live consumers; fake engines used for boundary unit tests contain no model/tool loop.
- [ ] AC12: Mandatory root/MCP/skill/delegation/continuation/recovery E2E scenarios and workspace gates pass on Rig-only builds. Current architecture docs match the sole-engine behavior and the retirement ledger accounts for every retained legacy-file symbol. No required migration capability is deferred while this spec is marked Shipped.
- [ ] AC13: Effectful built-in, MCP, skill-driven and delegated tool calls preserve the existing configured filesystem/network/working-directory/resource/time confinement, not merely actor authorization. The adapter accepts only server-derived scoped context and cannot widen it with model arguments, skill content or child configuration. Tests reject access outside the fixture's authorized path/network scope, preserve its working directory, and terminate long-running fixture commands at the configured deadline. MCP transport adapters retain configured credential/server scope; this criterion does not claim to sandbox a remote server beyond the existing supported controls.

## Assumptions

- Product: the user explicitly requests no legacy executor and Rig for MCP, skills, root/subagent orchestration and execution (user confirmation 2026-09-06). Subsequent confirmation authorizes incremental implementation and the tracking repair needed to start.
- Technical: the current selector is opt-in and falls back for MCP; all three construction sites use it (gateway/gateway-execution/src/invoke/executor.rs; runner/invoke_bootstrap.rs; runner/core.rs; delegation/spawn.rs).
- Technical: Rig 0.39.0 is pinned to 6b1991bfb246411dd75839c8611e801a2309d33c; the existing provider/tool adapters are retained starting points (Cargo.lock; runtime/agent-runtime/Cargo.toml). Exact hooks required for full parity are a mandatory T1 feasibility gate, not an asserted upstream capability.
- Technical: no docs/architecture/reference.md was found; design follows existing Rust/Tokio crate boundaries and runtime/gateway AGENTS files.
- Process: full code work-loop applies (structural and agent/MCP security boundaries). Scope and strategy are approved; no new external API contract or UI redesign is authored.

## Related work

[Rig Engine Migration](../rig-engine-migration/spec.md) is historical adapter/A-B migration context; its Shipped status does not establish Rig-only completion. This spec owns the outstanding hard cutover. The local gateway responsibility extraction and daemon-watch change are preserved, not silently bundled into the cutover's future implementation commits.
