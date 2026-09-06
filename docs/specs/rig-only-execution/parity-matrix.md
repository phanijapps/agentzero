# Rig-only execution: capability evidence

## Status

T1 is complete; T2 is in progress. These are feasibility results, **not** a completed cutover or
proof of production parity. The accepted starting commit is `e38e8003` on
`feat/rig-only-execution`; its working tree was clean before this slice.

The user approved small incremental implementation changes and the tracking
repair. The spec is registered under `ini-001`, and a code-mode workflow now
replaces the planning-only tracking. The user subsequently delegated cutover
dependency/implementation/plan decisions on this branch. The amendment permits
bounded bridge/transport repairs in T1 and native Rig MCP integration in T3;
both amendment reviewers returned Clean. Unrelated backlog entries were preserved.

## Pinned contract slice

Oracle: strong software (installed Rust source and compiler), with executable
behavior probes. Rig is pinned to version `0.39.0`, revision
`6b1991bfb246411dd75839c8611e801a2309d33c` in the runtime manifest and lockfile.
The probes compile and execute that locked dependency offline; no dependency
upgrade, provider credentials, network service, or user vault is needed.

| Capability | Evidence | Cutover disposition |
| --- | --- | --- |
| Before-request policy | `AgentHook::on_event(CompletionCall)` returns `Flow::override_request(RequestOverride)`. Probe observes the overridden system message and output limit at `CompletionModel::stream`. | T5/T6 can use this seam for supported per-request fields. |
| Preamble encoding | Rig folds the preamble into the leading `Message::System` in `CompletionRequest.chat_history`; it does not leave the override in `request.preamble`. The probe asserts the actual provider input. | Preserve the existing model bridge's system-message conversion. |
| Provider usage | `StepEvent::StreamResponseFinish` exposes `GetTokenUsage`; the probe observes 11 input / 3 output / 14 total tokens. | T5 must verify cumulative gateway events, including multiple turns and missing metrics. |
| Pending-stream lifetime | After the provider stream is actually polled, dropping the Rig run drops that stream within the probe's one-second bound. | A usable cancellation seam, not proof that AgentZero's stop flag or provider task is canceled. |
| History mutation | `RequestOverride` has no history replacement field; a model wrapper owns the request. A compiled probe removes an obsolete user/assistant pair before provider dispatch while preserving system/current messages. | T6 must prove persistent multi-turn compaction/checkpoints; the single-request seam is verified. |
| Tool hooks and hidden context | Existing adapter implements `RigExecutionHook`, `ToolCallExtensions`, and sequential tool dispatch. | Existing adapter tests are regression evidence; T4 still needs built-in/MCP/skill/connector end-to-end parity fixtures. |
| MCP lifetime | Existing manager starts and retains clients; stdio uses `kill_on_drop`. | T3 must establish session ownership and bounded shutdown for every transport; kill-on-drop alone is insufficient evidence. |

Upstream source slices: `crates/rig-core/src/agent/hook.rs` (`StepEvent`,
`RequestOverride`, `Flow`) and `crates/rig-core/src/completion/request.rs`
(`CompletionModel`, `GetTokenUsage`, `Usage`) at the pinned revision.
Local integration points: `runtime/agent-runtime/src/rig_adapter/engine.rs`
and `model.rs`.

## Concrete gaps discovered

### Configuration ownership inventory

All fields of `ExecutorConfig` (`executor.rs:176`) are accounted for below.
These are target owners, not completed parity assertions.

| Fields | Preserved behavior / disposition | Task |
| --- | --- | --- |
| `agent_id`, `provider_id`, `model` | Metadata, middleware identity, tool context and resolved provider | T2/T5/T6 |
| `temperature`, `max_tokens`, `thinking_enabled` | Prove actual provider payload, not merely duplicate config values | T2/T5 |
| `system_instruction` | Effective gateway-resolved instruction, including remote-peer specialization | T2/T6 |
| `tools_enabled`, `model_hidden_tools` | Separate advertisement from executable authorization | T4 |
| `mcps` | Explicit discovered server/tool bindings; no permissive name fallback | T3/T4 |
| `skills`, `conversation_id`, `initial_state` | Scoped ToolContext, loaded skills, ward/actor/planning state, recovery | T2/T4/T6–T8 |
| `max_tool_result_chars`, `offload_large_results`, `offload_threshold_chars`, `offload_dir` | Existing ToolResultContextConfig policy with confinement | T6 |
| `max_iterations`, `extension_size` | No production executor reads found; prove callers before retirement, do not recreate dead behavior | T2/T10 |
| `max_extensions` | Existing ProgressTracker input; distinguish advisory from actual extension | T5 |
| `context_window_tokens`, `compaction_warn_pct` | Prompt budget/warning and memory flush nudge | T6 |
| `turn_budget`, `max_turns`, `complexity` | Wrap-up and complexity nudges versus hard stop; test turn boundaries | T5/T6 |
| `before_tool_call`, `after_tool_call` | Deny dispatch and rewrite result without fabricating success | T2/T4/T5 |
| `tool_execution_mode` | Sequential/parallel behavior; shared per-call state must not race | T5 |
| `transform_context` | Authoritative message mutation before every provider request | T2/T6 |
| `single_action_mode` | Prevent extra side-effecting calls from a single response | T4/T5 |
| `rig_agent_config` | Required direct construction inputs replace optional selector gate | T2/T9 |

Additional executor-owned inputs: MiddlewarePipeline; RecallHook/RecallHookResult,
recall interval and initial dedup keys; SteeringQueue/SteeringHandle;
ProgressTracker; per-turn delegation flag reset and per-call action draining.
T6 must preserve peer-steering taint, respond-only tool restriction, external
argument/result redaction, and durable delivery acknowledgement **after** a
successful provider completion. Replay tool interception also needs a Rig
equivalent for Mode Full fixtures (T4/T10).

### Event and entry-point inventory

Confinement anchors for T4: gateway's `invoke/executor.rs` constructs the
actor-filtered registry and `RecallAuthorizationContext`; Rig tools receive
server-owned `SharedToolContext`, not model-supplied session identity.
`runtime/agent-tools/src/tools/connectors.rs::invoke_connector_capability`
passes `ctx.session_id()`/`ctx.agent_name()` into the provider, which remains
the downstream authorization boundary. `GatewayFileSystem` supplies roots,
not a sandbox. `tools/execution/skills.rs::resolve_skill_dir` joins configured
roots; root selection alone is not path confinement. T4 must exercise actual
denials and keep provider/path checks in their owners instead of treating a
catalog snapshot or directory prefix as an authorization proof.

All `StreamEvent` variants (`types/events.rs:16`) remain protocol contracts:

- T5: Metadata, Token, Reasoning, ToolCallStart, ToolCallEnd, ToolResult, Done,
  Error, TokenUpdate, Heartbeat, IterationsExtended.
- T5/T7: ShowContent, RequestInput, WorkSurface, WorkSurfaceUpdated,
  WorkSurfaceDeleted, SessionTitleChanged, ActionRespond.
- T5/T8: ActionDelegate.
- T5/T6/T7: ActionPlanUpdate, ContextState, WardChanged.

Current Rig already emits cumulative TokenUpdate (`rig_adapter/engine.rs:379`);
the stale module comment is not evidence of absent behavior. ToolCallEnd,
ToolResult's context/error/duration distinctions, heartbeat and final context
state need explicit parity tests. Do not synthesize IterationsExtended behavior
without finding a live producer. Respond arguments must survive both root and
continuation persistence, even when no text tokens are emitted.

The three production selectors are in `runner/invoke_bootstrap.rs:1222`,
`runner/core.rs:1623`, and `delegation/spawn.rs:721`. They feed initial stream,
continuation, and child execution respectively (T7–T9). WebSocket including
Research, HTTP webhooks, CLI, cron/hooks, inbound A2A, continuation watcher,
delegation dispatcher, live resume and persisted subagent resume must converge
on those same factories and be exercised through their real adapters (T11).
The public runtime execute/execute_stream/convenience factory callers and
tests must be migrated before those exports are removed (T10).

### Open implementation gaps

1. **Stop during silence:** `RigAgentEngine::run` checks the atomic stop flag
   only after `stream.next().await` returns. T5 must select cancellation while
   the stream is pending and prevent later tool dispatch.
2. **Provider task ownership — repaired:** a regression reproduced the leaked
   pending provider task. The production stream now owns and aborts that task
   on drop; the regression passes. T5 still must wire the gateway stop flag to
   stream cancellation while no items are arriving.
3. **Research baseline fixture:** `websocket/handler.rs` publishes
   `InvokeAccepted` immediately after successful durable enqueue, explicitly so
   Stop can cancel before bootstrap. The existing
   `research_subscribes_then_persists_before_acceptance` test instead asserts an
   empty channel before creating bootstrap rows. This is a source-confirmed
   mismatch with the current early-acceptance behavior, not evidence that Rig
   broke Research. The fixture now checks durable deduplication, absence of a
   bootstrapped session, immediate acknowledgement and exactly one response.
   The isolated corrected test passes and independent review returned Clean.
   Committed separately as `edcf854d`.
4. **Configured SSE discovery — repaired:** the loopback-client probe first
   failed for `sse` with a JSON-only decoding error. All three configured POST
   transports now share the existing JSON/event-stream response decoder; the
   170-line duplicate client was deleted. All three probes pass. This tests
   finite response decoding, not full session/old GET-SSE protocol conformance.

### MCP integration decision

The reviewed branch decision enables Rig's native `tool::rmcp::McpTool` with
exact SDK pin `rmcp =1.7.0`, sourced from crates.io and the official
`modelcontextprotocol/rust-sdk` repository. Rig remains at its original pin.
Explicitly filtered tools will be registered; the SDK's auto-registration
handler must not bypass actor authorization. Native call timeout abandons a
request locally, so session ownership still must close/cancel resources.

The feature tree includes upstream-required server/macros/base64 defaults even
though our direct dependency disables defaults. Enabled transports are child
process and Streamable HTTP. TLS uses `reqwest-tls-no-provider` to reuse the
existing ring provider, avoiding a second AWS-LC provider. Existing reqwest
0.12/0.13 and schema dependencies are reused; process-wrap/sse-stream are SDK
transport dependencies, not new AgentZero helper abstractions. Lockfile records
registry checksums. Baseline and initial post-addition audit/deny scans pass
under existing policy (including existing RUSTSEC-2026-0235 ignore and warnings);
final feature-selection scans and native lifecycle probes subsequently passed
(see Latest verification below).

## Verification log

- `cargo test -p agent-runtime --test rig_capabilities --locked --offline`:
  **2 passed**. The first compile exposed required serde bounds and typed empty
  history; corrected before the runtime probes. An initial preamble assertion
  exposed Rig's system-message folding; the corrected assertion checks the
  provider-visible message rather than weakening the policy requirement.
- `cargo clippy -p agent-runtime --test rig_capabilities --locked --offline -- -D warnings`:
  **passed** (includes type checking the runtime and test target).
- `cargo test -p agent-runtime --lib rig_adapter --locked --offline`:
  **34 passed**, including hidden context, tool-hook denial, delegation yield,
  provider bridging, history conversion and tool-state persistence.
- `rustfmt --edition 2021 --check runtime/agent-runtime/tests/rig_capabilities.rs`
  and `git diff --check`: **passed**.
- `cargo check --workspace --locked --offline`: **passed**.
- `cargo test -p gateway --lib research_subscribes_and_enqueues_before_early_acceptance --locked --offline`:
  **passed**, after reproducing the old test failure before editing.
- `cargo test --workspace --locked --offline`: **failed** at the pre-existing
  saved-surfaces envelope assertion (`gateway/tests/saved_surfaces.rs:66`).
  Tracked as `pre-existing-saved-surfaces-envelope`, not a passing full gate.
- `cargo check --workspace --all-targets --all-features --locked --offline`:
  **failed** because obsolete `adk-eval` imports the absent `adk_core` dependency.
  Tracked as `pre-existing-adk-eval-build`; T10 must address it, not add ADK back.
- `cargo test -p agent-runtime --test mcp_transport_capabilities --locked --offline`:
  **2 passed, 1 failed** (configured SSE discovery). Only discovery decoding is
  tested so far, not initialization/session/auth/call/cancel/shutdown parity.
- `bash e2e/scripts/boot-full-mode.sh simple-qa --fresh-vault`: an initial mock
  startup attempt timed out and cleaned up. A standalone mock health check
  passed; subsequent isolated harness runs booted. The final probe observed
  gateway `/api/health` **200** and UI `/` **200**, then tore down its temporary
  vault/processes. This used the existing baseline daemon binary and proves
  harness availability only, not rebuilt cutover behavior or UI agent journeys.
- Release build and full Mode Full agent journeys: **not run**, owned by T11.

## T1 completion

- The canceled-native-startup probe passes: native MCP lifecycle group 3/3.
  Bounded review's documentation corrections were applied.
- Recorded final T1 gate outcomes and advanced to T2. The inventory, request
  history seam, provider-task ownership, hidden tool context, four configured
  transport discovery paths, native stdio lifetime and isolated harness boot
  mechanism have concrete evidence above. Full transport failure matrices and
  production parity remain explicitly owned by T3–T11, not claimed here.

### Latest verification (after native MCP feature selection)

- `cargo test -p agent-runtime --lib rig_adapter --locked --offline`: 40 passed
  (including native stdio call/close and cross-session isolation). The three
  original capability probes now live under `src/rig_adapter/`; the earlier
  `--test rig_capabilities` commands describe the historical pre-move target.
- `cargo test -p agent-runtime --test mcp_transport_capabilities --locked --offline`:
  all three finite event-stream discovery probes passed after the decoder repair.
- `cargo clippy -p agent-runtime --lib --test mcp_transport_capabilities --locked --offline -- -D warnings`:
  passed. `cargo fmt --all -- --check`, boundary check and diff check passed.
- `cargo check --workspace --locked --offline`: passed with native MCP enabled.
- Final feature tree: 769 locked packages (baseline 738), existing ring is the
  sole TLS crypto provider. `cargo audit --db /tmp/zbot-rig-audit-TPbwTtpY/db --no-fetch --json`
  and `cargo deny --log-level error check`: exit 0 with existing policy unchanged.
  Existing unmaintained/unsound warnings and ignored advisory are not resolved
  by this migration; no new ignore or blanket suppression was added.

## Review disposition

Bounded independent review returned **Clean — ready to commit**. Its initial
documentation concern was resolved by explicitly distinguishing the original
planning turn from the subsequent implementation approval in `review.md`;
the sealed plan's task strategy was preserved.
No production engine has been retired yet. No AC is marked complete.
`project-knowledge not requested`; no capture residue was admitted at approval.
