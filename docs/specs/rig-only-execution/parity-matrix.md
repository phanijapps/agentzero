# Rig-only execution: capability evidence

## Status

T1–T6 gates are complete; T7 is in progress. These are incremental results, **not** a completed cutover or
proof of production parity. The accepted starting commit is `e38e8003` on
`feat/rig-only-execution`; its working tree was clean before this slice.

The user approved small incremental implementation changes and the tracking
repair. The spec is registered under `ini-001`, and a code-mode workflow now
replaces the planning-only tracking. The user subsequently delegated cutover
dependency/implementation/plan decisions on this branch. The amendment permits
bounded bridge/transport repairs in T1 and native Rig MCP integration in T3;
both amendment reviewers returned Clean. Unrelated backlog entries were preserved.

Review scope follows the user's later instruction: bounded minor reviews, not
broad security or quality sweeps. Specialist security-reviewer,
quality-engineer and whole-spec quality sweeps are named skips under that
instruction; behavioral/confinement tests, compiler gates and final real-artifact
verification remain required. No release parity is claimed by skipping a review.

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

### T2 execution record

- Owned change: shared config/hooks/errors move to `engine/{config,hooks,error}`;
  the old facade implementation moves into `executor.rs`, leaving the facade
  and Rig adapter independent of that module. Required contract tests move with
  their types; loop internals remain untouched for later parity/deletion.
- Direct construction: `PreparedExecution` carries resolved inputs only (no
  model/tool loop); Rig's factory consumes them without constructing the old
  executor. The intermediate selector creates the old implementation only in
  its existing branches until T9. Recall/steering setup and registry identities
  are retained in prepared inputs, not reconstructed after selection.
- Done evidence: full runtime tests, gateway construction tests, workspace
  check and real daemon/mock/UI smoke. The pure extraction passed 434 runtime
  tests with 2 existing ignores before direct construction integration.
- Not changing: default routing, MCP production dispatch, context policy or
  durable terminal handling in this slice. These remain T3–T9 responsibilities.
- Declined: a renamed executor loop or gateway service locator; neither is
  needed to carry construction inputs. The extraction is moved code, not
  retired execution behavior. The daemon smoke uses an isolated fresh vault.
- Smoke-discovered parity repair: the rebuilt Rig daemon rendered `4` but
  issued two model requests for the one-request `simple-qa` fixture, then showed
  an LLM error. The production factory regression independently reproduced the
  extra request. Stop polling after a successful respond action (not merely
  the tool name); preserve Done for respond and omit it for delegation yield.
  This bounded terminal-boundary repair is pulled forward from T5 to make the
  direct-construction smoke truthful. The strict replay assertion is retained.
- The repaired factory tests prove successful respond uses one provider call,
  preserves usage and emits one Done; denied respond permits recovery. All
  44 Rig tests pass. Runtime/gateway Clippy with `-D warnings` passes.
- Rebuilt daemon Mode Full: `ZBOT_ENGINE=rig ./node_modules/.bin/playwright test full-mode/simple-qa.full.spec.ts --reporter=line`
  from `e2e/playwright` **passed** (9.1 seconds), including zero replay drift,
  persisted answer `4` on a fresh document and no LLM-error banner. The fresh
  document must restore the harness gateway query parameters after SPA routing;
  the initial plain reload reached Vite's default backend, not the isolated daemon.
- One full parallel runtime run hit an unchanged tracing-capture assertion;
  the isolated test and full serial binary pass (436 passed, 2 existing ignores
  before the two terminal-boundary tests). No assertion was removed or weakened.
- Final T2 `cargo test -p agent-runtime --lib --locked --offline -- --test-threads=1`:
  438 passed, 2 existing ignores. The wave advanced after the runtime, gateway,
  Clippy, formatting/boundary and rebuilt full-mode gates passed.

Bounded independent review returned **Clean — ready to commit**. Its initial
documentation concern was resolved by explicitly distinguishing the original
planning turn from the subsequent implementation approval in `review.md`;
the sealed plan's task strategy was preserved.
No production engine has been retired yet. No AC is marked complete.
`project-knowledge not requested`; no capture residue was admitted at approval.

### T3 execution record

- Scope: replace spawn-per-call stdio with one initialized SDK session, use the
  actual Streamable HTTP protocol, retain configured HTTP/SSE POST semantics,
  and attach session cleanup directly to the Rig factory/engine lifetime.
  Actor inventory/dispatch remains T4; the selector is not removed early.
- Removed `mcp/stdio.rs` (436 lines, including its implementation-specific tests).
  Real subprocess fixtures now verify initialize/list/call, persistent PID,
  EOF, canceled startup, pending-call close, replacement, failed discovery,
  canceled cleanup, manager drop and session isolation. HTTP-family fixtures
  verify auth failure/redaction, cancellation and a virtual-clock 30-second
  call deadline without replay. Streamable HTTP additionally verifies the
  initialized notification, session ID and DELETE; it is not a bare POST alias.
- Native SDK contract findings: its HTTP worker awaits a POST outside its
  cancellation select, and its default client lacks a request deadline.
  `mcp/native_http.rs` delegates wire parsing to the SDK while applying request
  cancellation, bounded DELETE and redacted errors before SDK diagnostics.
  Reinitialization on expired sessions is disabled to prevent implicit replay.
- The wrapper requires direct edges to existing locked reqwest 0.13.4 and
  sse-stream 0.2 packages. Explicit rustls ring initialization fixes a reproduced
  no-provider panic, respecting an already-installed host provider. `tempfile`
  is test-only. Package count remains 769; no new package versions or backend.
  The boundary checker permits only the renamed MCP dependency in agent-runtime
  and only two MCP transport source files; three counterexample tests preserve
  the ban elsewhere. Audit/deny still pass under the unchanged baseline policy.
- Rig execution guards supervise close on success/error, dropped futures and
  never-run engine disposal, even while clients/managers remain referenced.
  Four production-factory subprocess tests pass. The first three failed before
  lifecycle wiring; the fourth reproduced out-of-runtime disposal and passed
  after capturing the owning runtime handle. Cleanup does not depend on Done.
- Verification: transport lifecycle 11/11, HTTP/SSE decoder probes 2/2,
  factory lifecycle 4/4, gateway construction 49/49, focused Clippy clean.
  Full serial runtime passed 441 tests with 2 existing ignores. The rebuilt
  daemon/real-UI strict simple-qa fixture passed in 8.8 seconds, including zero
  replay drift before/after reload. Additional real-fixture tests cover stdio
  initialization/call deadlines, HTTP-family EOF, native HTTP startup timeout
  and cancellation, and observed socket disconnection after canceled requests.
  Final review disposition is recorded below.
- Bounded-review finding applied: SDK trace events precede result redaction.
  A real native stdio trace-canary test reproduced the leak. The runtime and
  daemon's console/file subscribers now share a non-overridable metadata filter
  for raw rmcp/sse-stream diagnostics; AgentZero's safe operational warnings and
  application trace logs remain enabled. The passing test covers native stdio,
  Streamable HTTP responses and an SSE comment canary, with positive trace/warn
  controls and an explicit `rmcp::service=trace` directive. A captured parser-target
  warning separately pins the sse-stream filter even when that dependency's
  optional tracing feature is disabled. Daemon wiring is a
  necessary same-concern T3 correction, not a logging-system redesign.
- Final bounded re-review: **Clean — ready to commit.** Runtime 441/2 existing
  ignores, lifecycle 11/11, runtime/daemon Clippy and boundary/format checks pass.
  After the logging fix, the rebuilt daemon's strict UI smoke passed again
  (8.7 seconds, `RUST_LOG=trace` supplied); no production routing switch yet.
- Declined: SDK automatic tool discovery/registration, a second execution loop,
  and a general HTTP/provider replacement. Windows cmd.exe launch semantics are
  preserved but have not been executed on this Linux host.

### T4 execution record

- Scope: frozen configured MCP inventory and existing effectful tools dispatched
  by the production Rig factory. No selector change before T9; no new registry,
  execution loop, or stronger remote-server sandbox claim.
- Prepared MCP bindings retain the original client and raw tool name. Only the
  configured server IDs are discovered; normalized name collisions fail closed.
  Discovery failure removes the client and reports the existing startup failure
  once. Hidden/disabled tools and the shared planning gate prevent execution.
- Missing hidden host context now produces a typed denial before side effects.
  Schema hardening and replay interception are shared with existing owners;
  strict replay miss/drift cannot fall through to live execution. Fresh-process
  replay tests exercise the actual global store, not an injected replacement.
- Independent local verification: 54 Rig adapter tests pass, including native
  stdio execution, raw-name dispatch, a prepared binding surviving manager
  replacement without rerouting, and replay hit/strict/drift/lenient cases.
  Full serial runtime library: 447 passed, 2 existing ignores; gateway builder:
  49 passed (before effectful-tool fixes, to be rerun at the T4 gate).
  Core runtime/gateway Clippy passed before the final simplification/test add.
- Remaining before T4 completion: real skill/file/shell/connector side-effect
  fixtures and their bounded same-concern fixes, integrated gates and a minor
  review. No acceptance criterion or wave completion is claimed yet.
- Combined production Rig fixture now passes skill loading/resource tracking,
  native stdio MCP, shell working directory and ward-scoped file writes. The
  fixture exposed a dropped call ID: `raw_tool_call` populated `id` and
  `internal_call_id` but left `call_id` empty. Contract-acquisition T1 (strong
  Rust/source oracle): pinned Rig 6b1991b `streaming.rs:147` defines these fields;
  `agent/runner.rs:454` passes `call_id` to the tool hook. The existing bridge
  assertion failed with None vs Some(call_1), then passed after preserving the
  provider ID in that field. The real skill graph now retains call-0.
- Effectful fixes have red-to-green evidence: canonical skill-path containment
  prevents an outside-file canary read; shell environment/working directory use
  the configured filesystem; timeout cancellation terminates the directly owned
  PID. This does not claim arbitrary descendant process-tree sandboxing.
  Built-in tool suite: 158 passed. Connector negative/error-canary coverage is
  being tightened before the final T4 gate.
- Final effectful fixture suite: 5 passed, including separate enabled unknown
  connector/resource/capability requests, disabled requests with zero HTTP, and
  HTTP-500 auth-canary assertions over model-visible errors and captured logs.
  Failed HTTP responses now use a shared status-only diagnostic; successful
  response bodies and wire semantics are unchanged. Request diagnostics omit URLs.
- Integrated checks before review fixes: runtime library 447 passed/2 existing
  ignores; agent-tools 158; gateway-connectors 21; four-crate Clippy with warnings
  denied; formatting/boundary/whitespace clean; rebuilt daemon + strict real UI
  simple-qa/reload passed in 9.4 seconds.
- Bounded review found display-name MCP references could start a canonical-ID
  client but disappear from prepared inventory. Applying canonical-ID preparation
  plus a gateway-builder-to-Rig real-call regression. The shell cwd review item
  is being reconciled against the existing explicit public cwd option; no new
  OS sandbox claim or silently changed policy is accepted.
- Final review dispositions: canonical-ID preparation applied; the real
  gateway-builder/display-name/stdin-server Rig call regression passes, and all
  50 builder tests pass. The cwd concern was reconciled with the existing public
  explicit-cwd option: host filesystem defaults and hidden ward identity are
  preserved without inventing a new OS sandbox or removing that option.
  Bounded re-review returned **Clean — ready to commit.**
- After the alias fix: gateway Clippy, formatting/boundary/whitespace checks
  pass; rebuilt daemon strict UI smoke/reload passes again in 8.8 seconds.
  T4 is complete. Root/child factory routing and end-to-end recovery remain the
  later T7–T9 integration layers; no complete cutover AC is claimed yet.

### T5 execution record (in progress)

- Scope: Rig run cancellation, control propagation and event fidelity in the
  existing adapter; only proven shared result-policy helpers leave executor.rs.
  Tests drive stalled providers, tool side-effect counters, terminal event
  ordering and configured limits through Rig. T4 is committed as b1e34141.
- Preserve effective pause/extension semantics: current gateway handles and DB
  state own them; neither execution engine consumes a runtime pause flag. Do not
  introduce a new pause loop or treat diagnostic iteration fields as hard caps.
- Declined: another executor, a replacement provider/retry stack, concurrent
  side-effect dispatch, or dynamic request overrides that the provider bridge
  does not actually forward. Context middleware/recall/steering remains T6.
- Resolve-versus-surface record open: no new user decision is required. Any
  control/event discrepancy is resolved against existing behavior and pinned
  Rig contracts before T5 is marked complete.
- First bounded checkpoint: pre-set and stalled-provider stop tests reproduced
  one-second timeouts before the fix. Stop now returns Stopped without Done,
  aborts provider work and schedules MCP cleanup independently. Tests stop both
  at ToolCallStart (zero effects) and after the first result (no second effect).
  Metadata uses prepared host IDs even when optional Rig metadata is stale;
  a real ten-second pending heartbeat and terminal context export are covered.
- Pinned Rig's native cap does not have the configured one-based boundary. A
  per-run CompletionCall hook preserves existing hard limits exactly: limits
  1/2/4 permit 0/1/3 provider requests; disabled permits 53 requests. The native
  ceiling avoids `max_turns + 1` overflow. Single-action reduces the fixture's
  104 effects to 52 before Rig sees sibling calls. No second loop was added.
- Parent minor diff review: no open findings; simplified an unnecessary optional
  limit field. Independent post-simplification gate: 59 adapter tests passed,
  runtime Clippy with warnings denied passed; formatting clean. This is an
  intermediate T5 checkpoint, not wave completion. Tool error/result telemetry
  and final integrated T5 checks remain.
- Second checkpoint restores typed tool errors, raw-versus-context results,
  elapsed time, blocked-hook feedback, after-hook success flags and
  ToolResult → ToolCallEnd ordering through Rig. Existing result truncation and
  offload policy is reused. Runtime library gate: 461 passed, 2 existing ignores.
- Contract disposition: the pinned Rig runner rejects unregistered calls before
  its dispatch hook. InvalidToolCall → skip provides typed model feedback and
  a correlated attempted-call trace, with zero side effects. Rig abandons the
  remaining siblings in that invalid model turn; the recovery fixture proves
  matching call/result history, no invented sibling success, and a subsequent
  respond answer. This fail-closed SDK behavior is retained without name repair
  or a second per-call execution loop.
- Actual Rig tracing reproduced raw argument/result disclosure before host
  hooks. The existing mandatory diagnostic filter now rejects the observed
  `rig` and `rig_core` roots alongside MCP SDK roots; a positive application-log
  sentinel proves capture is active. No new logging layer was introduced.
- T6 still owns dynamic post-middleware/steering peer taint, context budgeting,
  recall and soft progress nudges. T5 proves shared visibility/guard plumbing;
  it does not claim that an unused marker establishes live context parity.
- Final integration gate after the diagnostic helper rename: 68 adapter tests,
  5 gateway real-tool fixtures, runtime/gateway/daemon Clippy (library, binaries
  and tests, warnings denied), formatting, unchanged boundary checker and its
  3 tests pass. Rebuilt daemon + strict real UI answer/reload passes in 8.7s.
  The shared filter is now named `safe_runtime_diagnostics`; SDK target names
  remain owned by the adapter metadata boundary. No compatibility alias remains.
- Bounded independent minor review returned **Clean — ready to commit**.
  T5 disposition record closed with no open review findings; T6 owns the
  explicitly identified context/soft-nudge integration, not deferred release
  capabilities. T5 control/event layer is complete.

### T6 execution record (in progress)

- T5 committed as 11567aac. Scope: focused adapter request-policy integration,
  preserving the existing middleware, skill/plan, recall and steering owners.
  Tests will drive multiple actual Rig provider requests, permanent compaction,
  full-fidelity initial history, budget refusal, cancellation during middleware,
  peer delivery acknowledgements and isolation across separately built sessions.
- Review shape: dependency-ordered checkpoints, starting with canonical context
  and per-request middleware/budget enforcement, then recall/steering and soft
  progress controls. Keep each checkpoint reviewable and the build working;
  wave completion requires the integrated T6 contracts, not only the first layer.
- Declined: copying the old iteration loop, changing the storage schema,
  introducing a provider transport, or widening skill/tool authority. A
  request-local rewrite alone is insufficient because Rig retains its own
  growing history; canonical host context must retain prior policy edits.
- Resolve-versus-surface record open: pinned history append and grouping are
  being checked before selecting the cursor. The original host history must
  preserve multimodal parts and summary flags; no lossy JSON round-trip is
  accepted as a checkpoint representation.
- Strong-tier pinned source contract (6b1991b): `AgentRun` owns input history
  separately from run-local messages (`crates/rig-core/src/agent/run/mod.rs`,
  with_history/messages/full_history); `CompletionCall` exposes history and
  prompt before provider request construction (`agent/prompt_request/streaming.rs:527`).
  Request construction may prepend/replace a system preamble, so its final
  length is not the canonical append cursor. Capture the pre-provider hook
  sequence and count grouped Rig messages before converting tool results into
  separate provider messages. Validate with multi-request runtime probes.
- Input-budget authority: gateway `resolve_effective_max_input` supplies
  `ExecutorConfig.context_window_tokens`; `max_tokens` is separately resolved
  output capacity. Do not subtract output twice. Count messages and offered
  tool schemas against the configured input limit; zero disables the check.
- Additional T5 follow-through: all 11 real MCP lifecycle/transport tests pass
  after renaming the diagnostic helper, including actual trace canaries.
- First checkpoint red evidence: production factory tests observe middleware
  invoked zero times across three Rig requests, pending preprocessing bypassed,
  and oversized messages reaching the provider instead of a typed rejection.
  The fix remains inside the engine-polled model future so its existing stop
  selection controls compaction as well as streaming.
- First checkpoint green: 75 adapter tests include seven context contracts;
  parent full runtime suite passes 468 tests with 2 existing ignores. The
  canonical context retains original summary flags and image/file parts, then
  appends grouped Rig tool deltas exactly once. Actual plan middleware sees host
  state. Both initial and later requests must fit the input budget.
- Parent minor review kept preparation transactional only in the simple sense:
  clone the last committed context and replace it on success. A canceled or
  failed middleware future no longer consumes the recoverable context. Live
  middleware events and typed failures are covered. No new transaction layer.
- Runtime Clippy, formatting and the unchanged boundary check pass. Rebuilt
  daemon strict real UI answer/reload smoke passes in 9.8s. This remains a T6
  checkpoint: recall, steering, soft controls and final context export are next.
- First context checkpoint committed as d5be5105; the five real gateway tool
  fixtures also pass with the request policy enabled.
- Second checkpoint scope: recall, steering, transform hooks and live peer
  authority at the same request boundary. Red probes observe zero recall calls
  instead of three, and a transformed peer marker permits two effectful tool
  executions instead of zero. Preserve middleware → recall → steering →
  sanitize → transform ordering, then final authority and budget checks.
  Peer delivery acknowledges provider success, never preparation or cancellation.
- Declined: muting user-visible peer-informed answers. Existing token/reasoning
  output stays visible; tool-event payloads and diagnostics retain their separate
  redaction rules. Snapshot export is a later T6 checkpoint, not a blind dump
  of canonical history into externally observable events.
- Snapshot seam acquired for the next checkpoint: ContextState is explicitly
  internal checkpoint data; gateway conversion returns None (pinned by a test)
  and selected-event log/trace handling omits it. Current root/continuation
  observers discard it, while write_turn_checkpoint synthesizes display fields.
  T7 can capture it privately without a new event API, preserving existing
  display keys and adding a namespaced runtime snapshot.
- Restore allowlist starts with skill:graph, skill:loaded_skills and app:plan.
  Do not restore planning gate/catalog/placeholder authority, agent/session IDs,
  ward/cwd or confinement: fresh bootstrap owns those. Keep checkpoints private;
  no new HTTP/WS/log/trace projection is authorized. Bounded retrieval found no
  checkpoint exposure path but did not claim an exhaustive admin-reader audit.
- Pinned Rig FinalResponse carries run-local history, excluding input history.
  Early respond/delegation exits occur before that item. A completed-pair tail
  can supplement canonical context, deduplicated at the next request boundary;
  never persist unexecuted siblings as completed calls. Preserve summary flags
  and media explicitly in the private snapshot representation.
- Provider-panic probe is red: closing the bridge channel after its task panics
  is interpreted by Rig as successful EOF. Observe the existing JoinHandle at
  channel close and return a bounded typed provider error; never log the panic
  payload or retry the request. Peer delivery must remain unacknowledged.
- Second checkpoint green: 84 adapter tests and all 8 live-context tests pass;
  parent full runtime suite passes 477 with 2 existing ignores. Provider success,
  failure, stop, dropped run and panic have explicit acknowledgment assertions;
  every non-success case has no Done. Late steering reaches the next request,
  scoped recall deduplicates, and final injected content is budget checked.
- Parent minor review: focused ContextInputs owns live injection; the canonical
  policy remains a coordinator. The existing provider JoinHandle is observed
  exactly once at stream EOF, with fixed error text rather than panic payload.
  Direct tool dispatch shares the host-owned peer flag with hooks and events.
  No open finding remains in this checkpoint.
- Runtime Clippy, formatting, whitespace and boundary checks pass. Rebuilt
  daemon strict real UI answer/reload passes in 9.6s. Soft controls, actual
  summarization and private context export remain the final T6 checkpoint.
- Second checkpoint committed as 374ba651. Final T6 checkpoint scope: existing
  progress scoring/advisory nudges and stuck safety, real summarization, and
  versioned private context export/restoration. Use focused owners; do not grow
  the context coordinator into another executor or import unused extension APIs.
- Resolved old warning-loop defect: executor.rs injects a memory-flush warning
  and immediately continues, consuming an iteration without a provider request
  despite its comment promising a turn to save facts. Rig will inject the nudge
  into the next actual request, not reproduce a dummy iteration. Existing hard
  limits still count real Rig request boundaries; a regression must prove the
  nudge is visible, one-shot and based on last prompt usage rather than billing
  totals. This is a bounded correction under delegated branch authority.
- Integrated compaction/recovery fixture found a missing mutable selection:
  LoadSkillTool uses skill:current_skill for relative resource reads, not the
  graph alone. The recovered call failed despite retained graph/loaded skills.
  Extend the explicit restore allowlist to this fourth key; it selects a skill
  within fresh configured roots and does not replace path confinement or actor
  authority. Fresh initial state still wins. Keep this real failing regression
  instead of changing it to an explicit-path call that bypasses the lost state.
- Minor review found that preserved image/file payloads were absent from the
  text-only token estimate. A real Rig regression with a small schema and a
  1024-token input limit reached the provider with oversized media. Count the
  serialized multimodal payload in the existing estimator; text-only counts
  stay unchanged. This estimates in-request content, not decoded image tiles,
  PDF pages or remote content behind a URL; provider-specific billing remains
  distinct. Both URL/base64 and image/file cases are covered.
- Parent real integration passes: actual ContextEditing, PlanBlock and
  Summarization middleware precede effectful Rig tools, with summary-provider
  calls and editing events asserted. Restored relative skill reads work; fresh
  ward/actor/session and plan win. File/skill traversal and unknown connector
  capability fail, network calls use the configured endpoint, and a shell
  command still times out after one second. Six real gateway tool tests pass.
- The known old executor trace-capture positive-control flake recurred only in
  the parallel full runtime suite. All 488 runtime tests pass serially with two
  existing ignores; T10 removes this obsolete loop test after preserving its
  neutral helper and Rig diagnostic assertions. No migration assertion relaxed.
- Final T6 gate rerun after both fixes: 489 runtime tests pass serially with
  two existing ignores; all six gateway effectful tool tests pass. Runtime and
  gateway-execution Clippy (lib/bins/tests), workspace formatting, whitespace,
  SDK boundary and its three regression tests pass. Rebuilt daemon's strict
  real UI answer/reload smoke passes in 9.8s. Final minor review is pending.
- Follow-up minor findings accepted: persisted FileRef attachments expand to
  base64 in the provider, so resolve them through the existing source helper
  before final budgeting and pass those resolved bytes onward. The regression
  previously reached the provider with an oversized real temporary file; both
  image/file cases now reject before dispatch. Missing references return a
  bounded typed policy error without the path. No remote URL fetch is added.
- Corrected prior discovery: needs_planning_nudge is live at executor.rs:1308,
  not an unused extension helper. Restore its one-shot advisory at the next
  real request after five planless tool calls. The actual Rig regression was
  red (zero advisories) before the fix. Unused extension APIs remain unported.
- Follow-up gates pass: 491 runtime tests plus two existing ignores (serial),
  six real gateway tool fixtures, runtime/gateway Clippy, formatting/whitespace,
  SDK boundary. Rebuilt real daemon UI answer/reload passes in 9.9s. The 50
  gateway builder policy/configuration tests also passed before the two focused
  fixes; no builder code changed. Bounded fix re-review remains pending.
- T6 bounded minor re-review: **Clean — ready to commit.** All concrete
  findings are resolved and verified. Private snapshot/progress owners plus
  integrated tests form the third dependency-ordered T6 checkpoint; T7 now
  owns persisting/restoring the private payload in the gateway runners.

### T11 baseline repair preparation (read-only)

- `pre-existing-saved-surfaces-envelope` is a stale fixture, not a handler bug:
  `gateway/src/http/surfaces.rs:41-83` returns wrappers containing execution_id,
  session_id, created_at and surface. UI transport types and QuickChat/Research
  consumers agree. The later repair must assert the wrapper identities and
  nested surface, not change the production response to a bare descriptor.
- Same contract drift exists in `contracts/openapi/work-surfaces.yaml:60-64`,
  `apps/ui/src/features/chat-v2/useQuickChat.test.ts:83-92` and
  `apps/ui/tests/e2e/persistent-surfaces.spec.ts:203-204`. Reconcile these fixture
  and documentation consumers when closing the baseline repair in T11.

### T7 runner preparation (read-only)

- Continuation and delegation invokers are captured before runtime setters
  install KG stores and ingestion/goal adapters. Unlike the shared model-registry
  handle, their plain Option clones remain stale. T7 must resolve these named
  dependencies at invocation time and remove duplicate bootstrap setter state;
  retain public setter APIs and the existing shared control registries.
- Root and continuation both need the same assistant-row resolver: a terminal
  respond answer can exist only in tool arguments. Persist it before completion
  and callback publication, with real-store tests for both entry points.
- Move continuation preparation out of core.rs, then consolidate observation
  and finalization with an explicit execution-mode policy. Keep root connector
  routing distinct and leave distillation/handoff scheduling unchanged. Do not
  hand handlers an Arc<ExecutionRunner> or add a generic service locator.
- The private versioned runtime checkpoint will replace display history on
  resume, not concatenate duplicate tapes. Fresh host instructions and authority
  remain authoritative; only the explicitly owned prior preamble may be removed.
  T7 persists this additive payload without changing existing display keys or
  exposing ContextState through gateway events.
- T6 completed in e3dfcc71; T7a starts with the live dependency fix and
  mechanical continuation extraction. Red proof uses the real runner harness:
  capture both invokers, install real Engram/episode stores through public
  setters, and observe the continuation's stale None. No checkpoint or stream
  semantics change in this first checkpoint.
- T7b must preserve messages that arrive after a checkpoint (especially child
  callbacks) and fresh recall/working-memory injections. Restoring the private
  tape cannot simply discard every supplied gateway history item. Use the
  durable message cursor rather than concatenating the full display tape.
- Existing CheckpointStore::latest sorts llm_turn before created_at, while a
  continuation creates a fresh handle counter. Make checkpoint ordering
  monotonic across the same execution ID; otherwise a shorter continuation can
  leave an older snapshot selected forever. Test this with the actual store.
- Parent extraction check: the complete invoke_continuation function body is
  byte-identical before and after the move (Git HEAD extraction versus new
  continuation_execution.rs, diff exit 0). The behavior change in T7a is the
  invocation-time shared integration snapshot, not a rewritten stream loop.
- T7a closure: parent bounded review removed repeated integration snapshots;
  all 609 gateway-execution library tests pass with Rig, Clippy is clean with
  warnings denied, and the rebuilt daemon passes the real UI answer/reload
  smoke (9.2 seconds). Formatting, diff hygiene and Rig boundary checks pass.
  This is an independently verified extraction checkpoint, not T7 completion;
  shared finalization and durable private-checkpoint recovery remain next.
- T7 recovery design: consume the runtime's typed private-restore seam before
  composing durable tail rows and fresh gateway recall/working memory. Never
  merge the full stored context object into fresh host authority. Preserve
  remote-peer history isolation and omit the current root row by durable ID,
  not matching text. A present malformed private snapshot must fail explicitly.
- Recovery persistence must distinguish successful durable writes from a
  batch-writer acknowledgement that merely attempted writes. Checkpoint query
  errors likewise cannot be treated as an absent checkpoint. The construction
  tests must cover these failure paths before recovery relies on either seam.
- Cursor review resolved: Rig emits ActionDelegate before ToolResult; child
  callbacks append directly while parent assistant/tool rows are batched. A
  callback can therefore precede the parent's final own row. Use a typed
  gateway-owned input high-water cursor plus this invocation's represented
  output IDs beside (never inside) the runtime snapshot. Replay after the input
  cursor, omit represented IDs, and advance the next input cursor across all
  scanned rows. This retains racing callbacks without duplicating own output.
  Include synthetic continuation prompt IDs in represented outputs, not in the
  input boundary. Tests cover both append orders and next-cursor advancement.
- T7 implementation closure: runner/recovery.rs owns the typed restore
  composition (agent_runtime::engine::snapshot::restore is now the public
  gateway seam). The stream stashes the final ContextState, flushes the batch
  writer before the checkpoint write, and records only append-confirmed row
  IDs as represented outputs (written_message_ids distinguishes durable
  success from the FIFO acknowledgement; a failed append stays unrecorded).
  The checkpoint carries the display JSON plus `app:rig_checkpoint` and
  `gateway_recovery` beside it. Continuation composes tape + tails, seeds the
  restored mutable keys via ExecutorBuilder initial state, and fails
  explicitly on checkpoint read errors and malformed snapshots (absent
  snapshots still display-replay for legacy-engine compatibility). Root and
  child prompt rows ride the represented set by durable ID, never text
  matching. CheckpointStore::latest now orders by created_at first (store
  test proves a newer lower-turn continuation beats a stale higher-turn root
  checkpoint), fixing the pinned-snapshot hazard flagged above.
- Parity-oracle correction, evidenced not assumed: parity_error previously
  asserted zero StreamEvents before an LLM error and parity_stop_cancel
  expected Ok with Done→TurnComplete on stop. The legacy executor itself
  emits StreamEvent::Metadata before any request (executor.rs:240) and
  surfaces ExecutorError::Stopped on user stop (executor.rs:728) — the T4-era
  Rig engine merely lacked both behaviors, so the assertions passed against
  a divergent engine. The corrected oracle permits only pre-request Metadata
  before an error, requires Err(Stopped) with exactly the pre-stop tokens and
  no Done/TurnComplete, matching the gateway's cooperative-stop finalization
  (execution_stream Err(Stopped) arm + trailing stop_execution publishes
  AgentStopped exactly once). No assertion was deleted to excuse a Rig gap.
- T7 gates: cargo check/test/clippy --workspace --locked green except the
  tracked pre-existing saved-surfaces envelope failure and the recorded
  agent-runtime tracing flake (passes in isolation at the T7 baseline);
  gateway-execution 620 library tests + 4/4 parity scenarios + recovery unit
  and integration suites (racing-callback both orders, cursor advancement,
  malformed snapshot, checkpoint read error, failed-append non-recording,
  fresh-root cursor/recording) pass. All-features gate still blocked by the
  tracked pre-existing adk-eval build entry.
- T7 structural follow-up (runner god-file decomposition, requested before
  finishing): core.rs shrank 2828 -> 581 lines with zero behavior change —
  the four inline test modules moved to `runner/core/*_tests.rs`, invoke
  entries + finish_initial_invoke to `initial_execution.rs`, live control
  wrappers to `session_control.rs`, resume_persisted_subagent to
  `subagent_recovery.rs`, spawn_delegation + invoker factories to their
  owning modules, TurnCheckpoint/write_turn_checkpoint to `recovery.rs`,
  and the ward session-index wrapper to `ward_artifact_indexer.rs`.
  Verification: the full 624-test inventory is name-identical to the
  pre-move tree (list diff empty), all suites green, clippy -D warnings
  and fmt clean, workspace check locked green. Test bodies are
  rustfmt-dedented copies (wrapper removal), not edits; the moved
  production code is byte-identical modulo import/visibility wiring
  (ExecutionRunner fields pub(super), one private fn -> pub(super)).
  invoke_bootstrap.rs (2631 lines) remains the next-largest file and is
  left for the T8/T9 waves that already touch it.

### T8 execution record

- T8 inventory: every local entry point already constructs through the one
  `select_engine` choke point — root invoke (initial_execution →
  invoke_bootstrap), continuation, delegation children (spawn.rs),
  durable Research and A2A ingress (services/runtime.rs → the same runner
  invoke entries), and persisted-subagent smart resume
  (subagent_recovery.rs → DelegationRequest → spawn). No second factory
  exists to remove; T8's work is proving the delegated flows behave.
- New flagship integration test (runner/core/delegation_flow_tests.rs):
  one scripted provider serves all three model turns — the root's
  `delegate_to_agent` (wait_for_result=true), the child's respond, and
  the parent's continuation answer. Asserts exactly one terminal per
  execution (the child's AgentCompleted precedes the root's — the UI keys
  subagent turns by execution id, so the child terminal routed to the
  parent session is legitimate and singular), DelegationStarted/Completed
  exactly once, callback-before-continuation durable ordering (user →
  tool call → delegated sentinel → callback → synthetic prompt → final
  answer), and the composed continuation request containing the callback.
- The flagship test exposed two real findings, both fixed in this wave:
  (1) on the Rig path the continuation failed closed on the input budget
  ("estimated 8644, limit 8192") — correct AC6 behavior; the shared
  harness fixture's provider window (8192) was simply too small for a
  full delegation continuation. Raised to 32768 (no gateway test relies
  on the old bound). (2) the test-support helpers were module-private;
  extracted to runner/core/test_support.rs for reuse.
- T8 gates: 625 library tests pass on BOTH the default (legacy) engine
  and `ZBOT_ENGINE=rig` (the established with-Rig suite run), clippy
  -D warnings and fmt clean, and the real daemon under ZBOT_ENGINE=rig
  passes the ward-archetypes (delegation archetype bundles) and
  stop-and-continue Mode Full specs (29.9s combined).

- T7 real-artifact verification: rebuilt daemon (cargo build -p daemon
  --locked) with ZBOT_ENGINE=rig passes the strict Mode Full simple-qa
  answer/reload smoke (9.7s) and the stop-and-continue regression
  (16.8s; the spec's cancel is best-effort-racy by design — first run
  failed on that race, two subsequent full runs passed both specs).
  The reload assertion exercises the flush-before-completion ordering on
  the shipped artifact, not only the unit gate.
- T7 bounded adversarial review (post-commit, all six focus areas
  verified clean): zero blockers; five findings applied in full —
  (1) present-but-unparseable gateway_recovery (or snapshot without
  cursor) now fails as continuation_cursor_invalid instead of silently
  defaulting to a zero cursor that would duplicate the conversation;
  (2) restored tapes strip previously injected `## Context Packet` recall
  packets so N-step delegation chains no longer accumulate N stale
  packets (fresh injection still happens each continuation);
  (3) spawn_continuation failures now crash the session and publish the
  terminal Error event instead of log-only hanging (integration test
  drives the watcher invoker and asserts SessionStatus::Crashed);
  (4) CheckpointStore module doc corrected to the created_at-first
  ordering; (5) restore input is now only the private snapshot — display
  fields like response/ward never enter engine initial state (unit test
  asserts only whitelisted mutable keys survive). Post-fix gates: 624
  gateway-execution lib tests, clippy -D warnings clean, both Mode Full
  daemon smokes pass again (27.9s combined).

### T10 retirement preparation (read-only)

- Most executor.rs behavioral tests now have Rig or neutral counterparts:
  control/steering, schema normalization, tool-result offload, stream behavior,
  and peer diagnostics. Preserve direct visibility-helper assertions in their
  neutral owner before deleting the old loop's test module.
- Port the remaining real gateway present_surface create/update and rejection
  flow tests before deleting their AgentExecutor fixture. Adapter surface-unit
  coverage alone does not replace gateway event conversion coverage.
- Gateway ward/template/catalog, middleware-order, effective-input-budget and
  MCP display-name alias tests remain useful. ExecutorBuilder already returns
  PreparedExecution, so these are not legacy-only tests merely because their
  containing file is named executor.rs. Keep the neutral policy tests; delete
  only selector/fallback expectations and actual old-engine construction.
