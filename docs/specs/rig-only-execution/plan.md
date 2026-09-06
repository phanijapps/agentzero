# Plan: Rig-only execution

- **Spec:** [spec.md](spec.md)
- **Status:** Drafting

## Approach

Close the capability gaps on the existing Rig adapter, replace construction and orchestration seams, then delete the legacy implementation. This is not a default-flag flip. The final artifact contains one engine, with no compatibility execution route. Work is staged in dependency-ordered reviewable commits on feat/rig-only-execution; intermediate commits are migration work, not shippable fulfillment of this spec.

## Constraints

- Baseline commit: 789fa55a. Branch created from fix/surface-time-window-attribution with existing dirty gateway responsibility extraction, package.json Rig watch setting, and workspace/doc edits preserved. Establish the exact accepted starting tree before implementation; do not commit or revert unrelated work implicitly.
- Follow runtime and gateway AGENTS boundaries. New modules below are focused internal owners, not new crates or facade-as-service-locator bundles.
- The behavior specs linked in spec.md's Constrained by header govern the named context limits/control, child role gating, handoff notes, delegation modes and provider configuration contracts. MCP OAuth is Archived and is cited only as provenance for preserving the current credential/privacy behavior, not as new feature scope. Historical storage paths and engine implementation details in these documents are not reinstated: current configured stores, durable messaging and this spec's Rig-only requirement take precedence. This migration preserves the named product contracts, not every historical implementation constraint.
- Keep the existing OpenAI-compatible transport, provider auth/retry/rate-limit stack, configured MCP transports, skill format, persistence and gateway protocol contracts. Reusing a protocol client does not constitute retaining the legacy execution engine.
- No wholesale distillation/knowledge architecture rewrite. Distillation remains a separate one-shot workflow; no AgentExecutor consumer may remain there or elsewhere. Typed extraction migration can be separate work.
- Approval is for the spec and plan separately. If the pinned Rig API cannot support the required semantics without a second loop, stop at T1 for an explicit version/design decision; do not invent an API or lower an AC.

## Construction tests

### Contract matrix

T1 produces a checked-in matrix mapping every ExecutorConfig field, hook, StreamEvent variant and execution entry point to a Rig owner, test and retirement disposition. MCP rows distinguish Stdio, SSE, Http and StreamableHttp/streamable-http; each has connect/list/call/auth-failure/cancel/shutdown cases. Connector rows cover connector_resource and connector_invoke with resource/capability scoping and denied side effects. Sources include runtime/agent-runtime/src/executor.rs, engine.rs, context_management.rs, middleware/, mcp/, rig_adapter/, gateway/gateway-execution/src/invoke/executor.rs and the three current select_engine call sites. Preserve existing tests as behavior oracles; capture sanitized fixtures before deleting legacy code. Future CI runs fixtures against Rig, never recompiles a legacy engine for comparison.

### Gates

Run affected focused tests on each task, then at integrated milestones:

```bash
cargo check --workspace --locked
cargo test --workspace --locked
cargo test -p gateway-execution --features test-stubs --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo check --workspace --all-targets --all-features --locked
cargo build -p daemon -p cli --release --locked
git diff --check
```

T1 validates these commands against actual supported feature/platform sets and records pre-existing blockers rather than weakening coverage. Run formatting checks over task-owned Rust files. Execute the existing real daemon/mock-LLM Mode Full harness at e2e/scripts/boot-full-mode.sh and e2e/playwright full-mode suites; T1 verifies actual setup and scripts before recording exact commands. Add missing MCP/skill/multi-agent scenarios to that harness. E2E is a cutover release gate, not a unit-test substitute.

Known baseline issue: websocket::handler::tests::research_subscribes_then_persists_before_acceptance fails at handler.rs:1417 on untouched 789fa55a. It is tracked as pre-existing-research-acceptance. Because Research ingress is required by this spec, triage it in T1 and correct a proven fixture issue or implementation defect in a separately reviewed change; it cannot be silently skipped for AC12.

## Design (LLD)

### Component / module decomposition

- **Neutral runtime contract:** engine.rs plus focused config/hook/error modules own AgentEngine inputs and events. Rig adapter imports no legacy executor module. Configuration describes desired execution, not one engine's internals. Traces AC1/AC11.
- **Rig construction:** evolve ExecutorBuilder into a neutral execution factory returning the Rig-backed engine directly. Resolve provider, authorized tools, skills, MCP session and context policy once. No build-AgentExecutor-then-deconstruct path. Traces AC1/AC9/AC10.
- **MCP session owner:** an explicit lifetime owner manages configured clients, tool descriptors, auth and cleanup. Rig-facing adapters dispatch approved MCP calls through that owner. Existing transport code is reusable; no separate MCP agent loop exists. Traces AC2/AC9.
- **Rig context/control integration:** focused adapters integrate current policy at request/turn/tool boundaries supported by the pinned Rig API. Rig remains the only iterative executor. Neutral controls carry cancellation, steering, iteration budgets and checkpoint state. Traces AC4–AC6.
- **Gateway orchestration:** focused preparation owners handle initial, continuation and recovery inputs; a shared stream observer/finalizer handles common persistence/event work. Explicit root/child/yield completion policies preserve different callbacks and routing. Existing narrow SessionSpawner/ContinuationSpawner/DelegationSpawner interfaces remain. Traces AC4/AC7/AC8/AC10.
- **Skills:** SkillService remains config/content authority; load_skill and related tools are Rig tools with the same scoped context. Skills are not a parallel execution engine and do not grant authority. Traces AC3/AC9.

### State & control flow

```text
Entry point → scoped session preparation → Rig execution factory
    → Rig model/tool loop (built-ins + MCP + skill tools)
    → AgentZero stream observer → durable messages/checkpoints
    → explicit completion / delegation-yield / stop / error policy
    → child callback and parent continuation through the same factory
```

The same registry identities and execution IDs flow throughout. Callback and final assistant rows are durable before publishing the dependent continuation/completion event. MCP resource cleanup happens on all exit paths independently of whether completion succeeds. Post-processing triggers retain existing policy; do not merge them with model/tool iteration.

### Failure, edge cases & resilience

No fallback engine. Failure to construct a required authorized capability produces a bounded, redacted error and cleans partial resources. Discovery of an unavailable optional capability follows its documented existing explicit-disabled/error contract, never silent capability omission. Provider retries remain bounded in the existing transport; side-effecting tool calls are not blindly replayed. Shutdown awaits resource cleanup with a 5-second upper bound and force-terminates owned subprocesses if graceful shutdown fails. Caller cancellation interrupts stalled streaming within 1 second in local deterministic tests. Policy and credential lookup errors deny before tool side effects.

### Data & schema

No database migration. Existing conversation rows, execution/session IDs, checkpoints, callback messages, skill/context state and tool-call correlation IDs remain readable. Move shared Rust DTOs only with live-consumer evidence; retain external serialized field names. No private vault data is copied into parity fixtures.

### Scope and retirement ledger

| Source | Final disposition |
|---|---|
| runtime/agent-runtime/src/executor.rs | Delete AgentExecutor, its loop/factory and legacy-only helpers; move only required shared contract types into neutral modules |
| runtime/agent-runtime/src/engine.rs | Keep facade; delete legacy impl; fake boundary test implementations remain loop-free |
| gateway/gateway-execution/src/invoke/executor.rs | Remove selector/A-B routing; refactor construction into direct Rig factory; preserve tool/config policy |
| runner/core.rs + execution_stream.rs + delegation/spawn.rs | Keep durable orchestration; unify common construction/stream-finalization, remove unused dependency forwarding |
| runtime/agent-runtime/src/mcp/ and llm/ | Retain needed transport/auth infrastructure; route MCP execution through Rig; delete only proven orphans |
| runtime/agent-runtime/src/middleware/ | Preserve policy via Rig integration, not a second executor loop |
| package.json, runtime/gateway AGENTS, active runtime/gateway README files and architecture docs | Remove ZBOT_ENGINE launch switch and fallback guidance at hard cutover |
| legacy-only tests/examples/features | Port behavior assertions; delete old implementation and A/B selector fixtures after coverage exists |

Report gross deletion, moved code, new adapter code and net non-test LOC separately. No arbitrary LOC target is an acceptance criterion.

## Tasks

### T1: Pinned Rig capabilities and the full parity matrix are proven

**Depends on:** none
**Touches:** docs/specs/rig-only-execution/*, gateway/gateway-execution/tests/rig_parity_tests.rs, runtime/agent-runtime/tests/*, e2e/*
**Verification mode:** goal-based probes and contract tests; no production stub at authoring time.
**Tests:** Compile/run minimal probes against the locked Rig revision for before-request context mutation, usage events, tool hooks, hidden context, interrupted pending streams, and owned MCP shutdown. Inventory every entry point/config field/event/hook, existing confinement helper and test. Run baseline gates, reproduce/triage the Research acceptance failure, and prove the E2E harness boots in an isolated vault. Covers AC1–AC13 test coverage.
**Approach:** Use contract-acquisition against pinned local source/toolchain before writing adapters. Store evidence and exact commands in parity-matrix.md. Map each gap to T2–T11; request a plan revision for an unsupported fundamental hook. Baseline-fixture repairs receive independent review, not edited expectations to bless a migration regression.
**Done when:** Each required capability has a verified integration seam and test owner; unresolved upstream/command feasibility blocks implementation, not merely release.

### T2: Engine contracts no longer depend on the legacy executor

**Depends on:** T1
**Touches:** runtime/agent-runtime/src/engine.rs, runtime/agent-runtime/src/executor.rs, runtime/agent-runtime/src/rig_adapter/*, runtime/agent-runtime/src/lib.rs, gateway/gateway-execution/src/invoke/*
**Verification mode:** TDD.
**Tests:** Config/hook/error API behavior and event serialization stay stable; Rig adapter builds without importing executor.rs; hidden tools stay hidden. Covers AC4/AC9/AC11.
**Approach:** Extract only shared contract types into neutral owners. Introduce direct Rig construction inputs; keep intermediate legacy callers compiling until T9, without adding new fallback paths.
**Done when:** Rig construction has no dependency on a constructed AgentExecutor or its module types.

### T3: MCP resources have a bounded, session-scoped lifetime

**Depends on:** T2
**Touches:** runtime/agent-runtime/src/mcp/*, runtime/agent-runtime/src/rig_adapter/*, gateway/gateway-execution/src/invoke/*
**Verification mode:** TDD plus real local transport integration.
**Tests:** Existing configured stdio/SSE/HTTP and distinct StreamableHttp (streamable-http) transports each connect, list and call against fixture servers; each has startup/auth-failure, EOF, timeout, canceled pending call and shutdown cases. No owned process/socket tasks remain after 5 seconds. Two sessions do not close each other's clients. Secret canaries stay out of model-visible fields and error/log channels. Covers AC2/AC9.
**Approach:** Give lifecycle responsibility to an explicit MCP session owner with async close and a cancellation-safe supervised cleanup path. Preserve current credential resolution and configured transport semantics; any unsupported actual transport is a blocker.
**Done when:** Real client/server fixtures prove successful calls and cleanup on each exit path.

### T4: Built-in, MCP and skill tools execute through the authorized Rig inventory

**Depends on:** T3
**Touches:** runtime/agent-runtime/src/rig_adapter/tool.rs, gateway/gateway-execution/src/invoke/*, runtime/agent-tools/src/tools/*, gateway/gateway-services/src/skills*
**Verification mode:** TDD plus Rig/provider/MCP fixture integration.
**Tests:** One real Rig turn calls a built-in, an MCP tool and load_skill; model-visible inventory and hidden context match each actor. Denied/missing tools/skills have zero side effects; forged session/ward/context args cannot replace server identity. Skill section bounds and loaded state survive subsequent turns. For shell/file/MCP/connector_resource/connector_invoke/skill-driven/child calls, assert server-derived working directory and scope, rejected paths and network targets outside the fixture's configured authorization, and bounded long-running command cleanup. Rig-turn connector negative cases prove model args cannot widen connector/resource/capability scope, unauthorized requests cause zero downstream calls, and errors/logs remain redacted. Inventory existing confinement helpers in T1 and demonstrate adapters preserve them rather than inventing a stronger remote-server sandbox claim. Covers AC2/AC3/AC9/AC13.
**Approach:** Adapt existing authorized tool implementations and MCP session handles into Rig dispatch. Keep SkillService and tool policy as authorities; no tool-registration bypass or broad agent credential inheritance.
**Done when:** Root and child actor fixtures execute allowed tools and reject disallowed calls through Rig, not just catalog snapshots.

### T5: Rig control and event behavior matches the execution contract

**Depends on:** T4
**Touches:** runtime/agent-runtime/src/rig_adapter/*, runtime/agent-runtime/src/engine.rs, gateway/gateway-execution/src/handle.rs, gateway/gateway-execution/src/events.rs
**Verification mode:** TDD.
**Tests:** Empty/text/reasoning/tool-error/respond-only streams, provider usage, iteration/token limits, pause/resume and extension; cancel a provider that never emits another item within 1 second; no next tool after cancellation; no final Done on delegation yield. Use deterministic event traces and side-effect counters. Covers AC4/AC5.
**Approach:** Wire neutral controls into Rig's verified seams; preserve sequential side-effect ordering unless independently justified. Separate stop/error/yield/final states and normalize usage/context events without synthetic success.
**Done when:** Control/event parity matrix is green with real Rig orchestration.

### T6: Context management, skills, recall and steering work on Rig

**Depends on:** T5
**Touches:** runtime/agent-runtime/src/rig_adapter/*, runtime/agent-runtime/src/middleware/*, runtime/agent-runtime/src/context_management.rs, gateway/gateway-execution/src/invoke/*
**Verification mode:** TDD plus multi-turn integration.
**Tests:** Small-budget conversations trigger context edits/summarization; system/plan/loaded-skill state survives; large results offload; every outbound request passes budget policy; deduplicated scoped recall and steering reach the next applicable turn; cancellation during compaction returns promptly. Recovered checkpoints reproduce context. Repeat confinement assertions after skill loading, compaction, steering and recovery: untrusted content cannot alter server-derived working directory, allowed path/network scope or command deadline. Covers AC3/AC5/AC6/AC8/AC9/AC13.
**Approach:** Integrate existing context policy at the T1-proven Rig boundaries. Do not copy the legacy execute_with_tools_loop. Test original skill/plan/recall security boundaries through the adapter.
**Done when:** Long-running Rig sessions retain context controls and do not leak state across two parallel sessions.

### T7: Root and continuation share durable stream finalization

**Depends on:** T6
**Touches:** gateway/gateway-execution/src/runner/*, gateway/gateway-execution/src/lifecycle.rs, gateway/gateway-execution/src/invoke/*
**Verification mode:** TDD plus real-store integration.
**Tests:** Root and continuation respond-tool-only answers persist before completion/reload; setup failures clean handles; live and persisted resume retain IDs; peer-only delivery and checkpoints work; no duplicate terminal event. Late-wired providers/stores are visible. Covers AC4/AC8/AC10.
**Approach:** Extract continuation preparation from core.rs and share stream observation/finalization behind explicit mode policy. Use the same registry identities and direct Rig factory for these paths. Leave memory/distillation scheduling behavior unchanged.
**Done when:** Both paths use common persistence/finalization and real-store parity tests pass.

### T8: Every local subagent and orchestration path uses the same Rig factory

**Depends on:** T7
**Touches:** gateway/gateway-execution/src/delegation/*, gateway/gateway-execution/src/runner/*, gateway/gateway-execution/src/agent_pool/*, gateway/src/durable_agent_tasks*, gateway/src/a2a_tasks*
**Verification mode:** TDD plus multi-agent integration.
**Tests:** All four delegation modes, nested/parallel children, wait_agent success/failure, concurrency permits, callback-before-continuation, child restart recovery and canceled-parent isolation. Exercise durable Research and incoming A2A tasks as local Rig executions. Covers AC1/AC7/AC8/AC9/AC10.
**Approach:** Route child and recovery construction through the same factory; retain durable parent/child policy and narrow invoker traits. Preserve outbound peer protocol semantics; remote peers are outside local engine enforcement.
**Done when:** End-to-end root → child → callback → continuation and restart fixtures run Rig throughout.

### T9: Rig is the unconditional engine on every supported entry point

**Depends on:** T8
**Touches:** gateway/gateway-execution/src/invoke/executor.rs, gateway/gateway-execution/src/runner/*, gateway/gateway-execution/src/delegation/*, apps/*, package.json
**Verification mode:** goal-based routing audit plus integration/E2E.
**Tests:** Default/debug/release and supported features, with ZBOT_ENGINE absent/rig/legacy/garbage, all execute Rig. CLI, cron, connector, HTTP/WS and durable/A2A ingress tests observe an actual Rig model/tool turn. Covers AC1/AC12.
**Approach:** Delete selector and engine flag handling, replace every legacy construction call site, and remove the watch prefix introduced in the previous task. Fail explicitly on unsupported required configuration, never fall back.
**Done when:** One engine is reachable from every production entry point; launch scripts need no opt-in.

### T10: No legacy executor or legacy-only compatibility code remains

**Depends on:** T9
**Touches:** runtime/agent-runtime/src/executor.rs, runtime/agent-runtime/src/lib.rs, runtime/agent-runtime/src/engine.rs, runtime/agent-runtime/tests/*, gateway/gateway-execution/tests/*, gateway/examples/*, Cargo.toml, Cargo.lock, tools/*, scripts/*
**Verification mode:** goal-based retirement audit plus retained behavior tests.
**Tests:** Workspace/default/all-target/supported all-feature builds and tests; static audit forbids AgentExecutor, select_engine, legacy execute_with_tools_loop and executable ZBOT_ENGINE branching in source/tests/examples/scripts. Review trait implementations and factory graph to catch renamed loops. Historical docs may mention removed symbols; fixtures may set the obsolete env only to assert it has no effect. Covers AC1/AC11.
**Approach:** Remove old loop/factory/exports/config-only helpers and A/B selector tests; port useful tests to Rig and neutral contracts first. Delete only dependencies proven unused by all remaining clients. Update parity capture tools so they need no compiled legacy implementation. Inventory legacy instructions in all active runtime/gateway README files for T11; historical frozen specs/RFCs may retain old symbols as history.
**Done when:** Retirement ledger has a disposition and evidence for every legacy symbol, no active legacy loop remains even behind test features, and compiler/behavior gates pass.

### T11: Rig-only release evidence and architecture documentation are complete

**Depends on:** T10
**Touches:** docs/architecture/*, docs/specs/rig-only-execution/*, runtime/AGENTS.md, runtime/agent-runtime/AGENTS.md, runtime/**/README.md, gateway/**/README.md, gateway/gateway-execution/AGENTS.md, gateway/gateway-execution/src/runner/AGENTS.md, e2e/*
**Verification mode:** full mechanical gates and real artifact E2E.
**Tests:** All construction gates, Mode Full root+MCP+skill+delegation/continuation+cancel/restart scenarios, isolated entry-point integration matrix, release binaries and no-legacy audit. Verify rendered UI final answers and persisted reloads, not only backend traces. Covers AC1–AC13.
**Approach:** Record commands/results/fixture versions/engine evidence; update active docs, including runtime/agent-runtime/README.md and other runtime/gateway READMEs, to the sole-engine truth while linking historical migration records. Audit active documentation for stale fallback/default-legacy/opt-in guidance; frozen historical specs/RFCs are explicitly excluded. Independent adversarial, security and whole-spec quality reviews must be clean. No skipped core parity case can be called Shipped.
**Done when:** Every AC, including AC13 confinement preservation, is demonstrably met, not deferred, and docs and deletion ledger match the final tree.

## Rollout

No database migration or new hosted infrastructure. Build and verify on the branch; release only the complete T11 artifact, never the interim dual-engine tree. Deployment is a normal binary replacement after active sessions are drained or durably checkpointed. Rollback is redeployment of the previously released binary using compatible persisted data, not an engine toggle in the new binary. No push, PR publication, deployment or cutover implementation is authorized by this planning turn.

## Risks and review shape

MCP cleanup, interrupted streams, context hooks, delegated identity and terminal-answer persistence are the highest-risk seams. This is DEEP work, split into the ordered layers above. Each task targets under 2,000 reviewable behavior/test lines; if T1 predicts a larger task, split its contract further and re-review before approval. Legacy deletion may have a large mechanical diff: retain an exact symbol/disposition ledger, green recompilation and ported tests rather than mistaking deletion volume for reasoning volume.

## Resolve versus surface

- Resolved: user means Rig, not Zig, and explicitly rejects a legacy executor/fallback.
- Resolved: preserving protocol/storage/policy owners is compatible with Rig-only model/tool execution; a second engine is not.
- Resolved: this turn creates a branch and Draft artifacts only; it does not authorize implementation or claim parity.
- Gate before implementation: pinned API feasibility and the accepted baseline tree are checked by T1; any necessary version/contract change requires a reviewed plan revision.
- Declined: swapping all provider transports, moving all memory to Rig, replacing durable orchestration with a new framework, and rewriting distillation. Each broadens scope without being required to eliminate the old executor.

## Changelog

- 2026-09-06: Initial Rig-only cutover plan after user clarification; no legacy fallback is an explicit release condition.
