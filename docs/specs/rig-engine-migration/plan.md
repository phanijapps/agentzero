# Plan: Rig Engine Migration

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done (T1-T16 complete)

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

Migrate by putting Rig behind the existing gateway-facing execution facade first, then progressively moving provider, config, tool, hook, stream, memory, and compaction behavior behind that adapter. The gateway, UI, memory stores, knowledge graph, and config files stay authoritative. The first task captures a parity baseline from the current conversation database as a gitignored exact local DB signature, a committed coarse old-engine signature, an old-engine event-path signature, and synthetic E2E cases, so later tasks can prove compatibility without committing private data. Only after the Rig-backed path is green do we rehome `zero-stores*`, remove active `zero-*` framework crates/imports/dependencies, and update active architecture docs.

## Constraints

- [`runtime-context-control`](../runtime-context-control/spec.md): live context control, prompt-cache, tool-pair, plan-block, and durable memory invariants must remain stable unless both specs are updated.
- [`mcp-oauth`](../mcp-oauth/spec.md): MCP OAuth token storage, dynamic client secrets, no-secret responses, and runtime bearer injection remain separated from model-visible config/tool arguments.
- [`subagent-role-gating`](../subagent-role-gating/spec.md): actor capability policy remains a Rust-enforced executable-tool boundary.
- [`agent-handoff-notes`](../agent-handoff-notes/spec.md): steering and handoff target validation remain current-session scoped.
- [`builder-delegation-hygiene`](../builder-delegation-hygiene/spec.md): delegation mode parsing, inference, propagation, and mode-specific executor rules remain intact.
- [`simplified-provider-model-configuration`](../simplified-provider-model-configuration/spec.md): existing provider/model configuration remains the user-facing control plane.
- `docs/research/rig-engine-migration.md`: adapter-first migration, preserve gateway/UI/memory/knowledge contracts, retire framework crates last.
- `memory-bank/websocket-events.md`: existing inbound websocket controls, direct `ServerMessage` responses, and `GatewayEvent` -> `ServerMessage` behavior are the event contract under test.
- Real conversation database content is private runtime data and must not be committed.

## Construction tests

**Integration tests:**
- Current DB parity harness against an external snapshot path, producing a gitignored exact old-engine DB signature, committed coarse old-engine DB signature, sanitized old-engine event-path signature, and committed synthetic fixtures.
- REST invoke/session and websocket `ClientMessage` control tests for inbound compatibility.
- Full gateway wire mapping tests for every current non-deferred `ServerMessage` variant.
- Gateway execution tests for delegation, continuation, stop/cancel, actor tool policy, and handoff/steering ownership.
- Delegation mode tests for `DirectArtifact`, `WardHygiene`, `WardBackedBuild`, and `StepExecutor`.
- Runtime adapter tests for Rig stream/tool/hook/error mapping into existing `agent_runtime` types.
- Store conformance and schema compatibility tests after `zero-stores*` rehome/rename.

**Manual verification:**
- After implementation, run daemon/UI or CLI against a new database and verify normal chat, streaming, first-party tool use, delegation continuation, and session reload.

## Design (LLD)

### Design decisions

- Adapter first: `gateway-execution` continues to call an AgentZero execution facade and receive AgentZero `StreamEvent`s. Traces to: AC2, AC5, AC15.
- Preserve configs: current agent/provider/MCP/OAuth/skill/connector/tool settings are mapped into Rig, not replaced at the user boundary. Traces to: AC4, AC8, AC9.
- Preserve event contract without a new `contracts/` tree: existing Rust event enums and `memory-bank/websocket-events.md` are the contract surface. Traces to: AC14, AC15.
- Cleanup is part of done: active `zero-*` and `zero-stores*` crates/imports are removed only after the Rig-backed runtime and store compatibility paths are proven. Traces to: AC16, AC17, AC18.

### Data & schema

- No planned durable schema ownership change for `conversations.db` or `knowledge.db`.
- The parity baseline references or snapshots the current conversation DB outside the repo.
- The committed old-engine DB signature artifact records deterministic coarse buckets, category presence, and schema hashes only; the exact DB-derived signature is written only under the gitignored `.rig-parity/` directory.
- Synthetic E2E fixtures contain sanitized session/message/event patterns needed to prove behavior.
- Rig `ConversationMemory` adapters may load/append through existing store traits but must not become a separate durable source of truth.
- `zero-stores*` crate names are transitional; storage behavior and schema compatibility are preserved while crate/package/import names are rehomed.

### Interfaces & contracts

- Gateway-facing execution interface: `AgentExecutor` or a direct successor remains the facade consumed by `gateway-execution`.
- Inbound gateway interface: REST invoke/session payloads, `ClientMessage` controls, direct `ServerMessage` responses, subscription behavior, and pause/resume/cancel/end semantics remain stable.
- Event interface: Rig output is adapted into existing `agent_runtime::StreamEvent`, then existing conversion produces `GatewayEvent` and `ServerMessage`.
- Config interface: existing YAML/JSON settings remain accepted; mapping into Rig is internal.
- Tool interface: current first-party and MCP tools are bridged into Rig tools while preserving executable filtering, hidden context, raw result, and context result behavior.
- Store interface: current store traits and SQLite behavior are retained while crate/package names are removed from the `zero-*` namespace.

### Component / module decomposition

- `runtime/agent-runtime`: owns `rig_adapter`, provider adapter, stream adapter, hook compatibility, memory adapter, and the facade used by gateway execution.
- `runtime/agent-tools`: moves first-party tools from `zero_core::Tool` implementations to Rig-compatible tools or transitional wrappers.
- `gateway/gateway-execution`: keeps runner, delegation, continuation, lifecycle, and event conversion responsibilities; changes should be limited to executor construction and compatibility tests.
- `gateway/gateway-services`: keeps agent/provider/config loading and feeds mapped settings into the runtime adapter.
- `stores/*`: rehome/rename `zero-stores*` crates while preserving traits, domain types, SQLite implementation, conformance harness, and schema compatibility.
- `framework/*`: removed, merged, or reduced during cleanup once no active dependency remains.
- `docs` and `memory-bank`: updated so active architecture describes the Rig-backed engine and no longer presents `zero-*` as current framework or persistence architecture.

### State & control flow

1. Gateway accepts the same REST/websocket inputs as today.
2. Gateway loads session, agent config, provider config, tools, MCPs, skills, connectors, and history as it does today.
3. Gateway builds the AgentZero execution facade.
4. The facade maps config/history/tool inventory into Rig `AgentBuilder`/`AgentRunner`.
5. Rig drives model/tool turns.
6. Adapter maps Rig streaming/tool/hook events into `StreamEvent`.
7. Existing gateway stream processing maps `StreamEvent` into `GatewayEvent`, side effects, persistence, and response accumulation.
8. Existing websocket router maps `GatewayEvent` into `ServerMessage`.
9. Delegation, delegation mode, continuation, stop/cancel, steering, and handoff remain gateway-owned and resume through the facade.

### Behavior & rules

- Actor policy is applied before executable tools reach Rig.
- Tool hidden context is carried through Rig extensions or equivalent typed runtime context, not model-visible arguments.
- Request overrides may narrow active tools or sampling but may not widen actor permissions.
- Tool result rewriting may change model-visible context only; raw persistence and UI payload behavior must remain intentional and test-covered.
- Runtime context control invariants from `runtime-context-control` remain binding; behavior changes require spec updates before code changes.
- `zero-*` compatibility shims may exist only during intermediate tasks and must be removed before the cleanup acceptance criteria are met.

### Failure, edge cases & resilience

- Missing current DB path should fail the parity setup with instructions to provide `ZBOT_PARITY_DB`, not silently skip parity.
- Private DB content must be sanitized before any committed fixture is written.
- Sanitization is positive-allowlist based and fail-closed: unhandled columns, JSON fields, blobs, tool payloads, connector/vault/user-data fields, or sanitizer errors abort before any committed artifact is written.
- `ZBOT_PARITY_DB` and fallback DB paths are canonicalized before opening; ambiguous discoveries, disallowed roots, and symlink escapes fail closed.
- Old-engine parity compares deterministic dimensions only: committed artifacts use coarse buckets, category presence, schema hashes, and sanitized event-path shapes; exact DB-derived event order, role sequence, tool-call/result pairing, delegation/continuation transitions, lifecycle state transitions, and shape hashes are kept only in the gitignored local DB signature.
- Synthetic fixtures are mandatory for simple chat, tool call/result, delegation/continuation, error, and stop/cancel. A fixture may be skipped only when the source behavior does not exist in the current engine and the skip is recorded in the spec or plan changelog.
- Rig provider/tool errors must map into existing executor error behavior and gateway error events.
- Stop/cancel must abort or terminate Rig execution without leaving orphaned delegations.
- Delegation completion races must preserve the existing order: persist parent callback before marking delegation complete and publishing continuation readiness.
- If Rig memory append fails after a run, execution should follow the existing warning/degraded behavior unless the spec is updated.

### Quality attributes (NFRs)

- Compatibility: inbound protocol, event order, and payload semantics remain stable for existing UI reducers.
- Operability: parity harness output identifies which session/event path changed.
- Security: hidden runtime context, OAuth tokens, connector auth scopes, and store handles are not exposed to model-visible tool arguments or persisted transcripts except where already intended.
- Maintainability: no active duplicate `zero-*` and Rig engine/persistence layers remain after cleanup.

### Dependencies & integration

- Rig source is available at `rig checkout`, commit `6b1991bf` during analysis; implementation pins the Cargo dependency deliberately before adapter work begins.
- Existing OpenAI-compatible provider stack should be adapted first; native Rig providers are optional follow-up only after compatibility.
- Existing store traits and SQLite implementations remain the durable persistence integration while names are rehomed away from `zero-stores*`.

## Tasks

### T1: Parity baseline captures old-engine signatures

**Depends on:** none

**Touches:** `runtime/agent-runtime/tests/*`, `gateway/gateway-execution/tests/*`, `docs/specs/rig-engine-migration/*`, optional `scripts/*`

**Tests:**
- Goal-based: a read-only parity setup command locates `ZBOT_PARITY_DB` or the current discovered DB, canonicalizes the path, rejects ambiguous/disallowed/symlink-escaped inputs, and emits a sanitized old-engine signature artifact without mutating the DB. Verifies AC1, AC22.
- Goal-based: committed synthetic fixture generation uses schema-aware positive allowlists, redacts before write, and refuses raw private message text, tool payloads, connector/vault/user data, unhandled JSON/text/blob fields, or sanitizer errors. Verifies AC1, AC22.
- Goal-based: sentinel-secret tests prove raw private strings cannot appear in signatures or fixtures. Verifies AC22.
- Goal-based: `cargo run -q -p gateway --features rig-parity-capture --example rig_parity_event_capture -- docs/specs/rig-engine-migration/fixtures/old_engine_event_signature.json` captures sanitized signatures from the current `StreamEvent` -> `GatewayEvent` -> `ServerMessage` path. Verifies AC1, AC15.
- Goal-based: committed signature artifacts name deterministic comparison dimensions and include only non-identifying source labels plus coarse provenance; exact source DB path, file size, modified time, exact DB counts, and exact DB-derived hashes are written only to gitignored local artifacts. Verifies AC1, AC23.

**Approach:**
- Add a small read-only parity probe/harness that accepts `ZBOT_PARITY_DB`.
- Default documentation to `ZBOT_PARITY_DB` and fallback discovery, without committing exact local DB path, size, or modified time.
- Generate sanitized signatures from old-engine behavior and DB-derived session patterns using positive allowlists only.
- Capture sanitized signatures from the current old-engine `StreamEvent` -> `GatewayEvent` -> `ServerMessage` path using deterministic, redacted scenarios.
- Write exact local DB provenance to a gitignored local manifest and the exact DB-derived signature to a gitignored local file, not committed artifacts.
- Derive mandatory sanitized synthetic scenarios: simple chat, tool call/result, delegation/continuation, error, and stop/cancel.
- Document where the external DB snapshot lives and how to regenerate synthetic fixtures.

**Done when:** the repo contains a coarse sanitized DB-derived artifact, an old-engine event-path artifact, and synthetic fixture artifacts, while exact DB provenance and exact DB-derived signatures remain only in gitignored local files.

### T2: Rig dependency is pinned deliberately

**Depends on:** T1

**Touches:** `Cargo.toml`, `Cargo.lock`, runtime crate manifests

**Tests:**
- Goal-based: `cargo metadata` shows the selected Rig package source and version/Git revision. Verifies AC3.
- Goal-based: the lockfile records the intended Rig dependency and no unintended broad dependency upgrade. Verifies AC3.
- Goal-based: `python scripts/rig_boundary_check.py` proves no workspace crate outside `agent-runtime` directly depends on Rig, no gateway/runtime source outside the adapter imports Rig, no source imports Rig-native providers, and no workspace crate directly requests `reqwest 0.13`. Verifies AC2, AC3, AC4.
- Goal-based: `cargo deny check` accepts the pinned Rig Git source, license set, and dependency policy with only documented pre-existing advisory exceptions. Verifies AC3.

**Approach:**
- Decide whether to depend on published Rig crates or a pinned Git revision.
- Add Rig only to the runtime adapter boundary first.
- Record the source/version decision in this plan changelog if it differs from the analysis checkout.

**Done when:** Rig is available to runtime code through a pinned, reviewable dependency.

### T3: Gateway-facing executor facade is isolated

**Depends on:** T2

**Touches:** `runtime/agent-runtime/src/executor.rs`, `runtime/agent-runtime/src/lib.rs`, `gateway/gateway-execution/src/invoke/*`, `gateway/gateway-execution/src/runner/*`

**Tests:**
- TDD: facade tests prove gateway can execute through the same public method shape and receive `StreamEvent`s. Verifies AC2.
- Goal-based: gateway-execution tests still compile without importing Rig types. Verifies AC2.

**Approach:**
- Define the narrow engine boundary currently represented by `AgentExecutor`.
- Keep Rig types out of gateway crates.
- Prepare the runtime for a Rig-backed implementation behind the facade while preserving existing behavior.

**Done when:** gateway execution depends on an AgentZero facade, not on implementation-specific engine internals.

### T4: Provider and agent config mapping works through Rig

**Depends on:** T3

**Touches:** `runtime/agent-runtime/src/llm/*`, `runtime/agent-runtime/src/executor.rs`, `gateway/gateway-execution/src/invoke/executor.rs`, `gateway/gateway-services/src/providers.rs`, `gateway/gateway-services/src/agents.rs`

**Tests:**
- TDD: current provider/agent config maps into Rig-compatible model settings without losing provider ID, model, base URL, API key reference, temperature, max tokens, thinking settings, provider params, or simplified-provider defaults. Verifies AC4.
- Goal-based: existing provider config tests continue to pass. Verifies AC4.

**Approach:**
- Implement an AgentZero `CompletionModel` adapter over the current OpenAI-compatible client stack first.
- Map existing agent instructions and runtime settings into Rig builder/runner options.
- Leave Rig-native providers as optional later cleanup, not the first compatibility path.

**Done when:** existing provider and agent config files drive Rig-backed execution without user-facing format changes.

### T5: MCP, skill, connector, and tool settings survive the adapter

**Depends on:** T3, T4

**Touches:** `runtime/agent-runtime/src/mcp/*`, `runtime/agent-runtime/src/tools/*`, `runtime/agent-tools/src/**/*`, `gateway/gateway-services/src/mcp*.rs`, `gateway/gateway-services/src/connectors*.rs`, `gateway/gateway-execution/src/invoke/executor.rs`

**Tests:**
- TDD: MCP configs, including OAuth-authenticated remote servers, map through the adapter without exposing bearer tokens or dynamic-client secrets. Verifies AC8.
- TDD: runtime MCP calls still inject bearer tokens from storage rather than from `mcps.json` or model-visible arguments. Verifies AC8.
- TDD: skill loading/listing, connector configs, connector auth scopes, and tool settings produce the same executable inventory and UI/API payload shapes. Verifies AC9.
- Goal-based: existing MCP OAuth and connector/service tests continue to pass. Verifies AC4, AC8, AC9.

**Approach:**
- Keep MCP/OAuth and connector config ownership in gateway services.
- Feed only executable tool definitions and hidden runtime context into Rig.
- Preserve current skill loading and tool settings filters before tool registration.

**Done when:** non-provider config surfaces survive Rig execution without user-visible format or secret-handling changes.

### T6: Tool bridge preserves policy, context, and result semantics

**Depends on:** T3-T5

**Touches:** `runtime/agent-runtime/src/tools/*`, `runtime/agent-tools/src/**/*`, `gateway/gateway-execution/src/invoke/executor.rs`, `framework/zero-tool/*`, `framework/zero-core/*`

**Tests:**
- TDD: first-party tool schemas become Rig-visible tool definitions while executable filtering remains actor-kind controlled. Verifies AC7.
- TDD: hidden runtime context is available to tools without appearing in model-visible arguments. Verifies AC10.
- TDD: raw result, context result, persisted session output, and UI event payload remain distinct. Verifies AC11.
- Goal-based integration: root, delegated executor, delegated reviewer, and ward-agent tool access tests pass. Verifies AC7.

**Approach:**
- Build a transitional bridge from existing `zero_core::Tool` implementations to Rig tools.
- Move first-party tools to direct Rig-compatible implementations where practical.
- Use Rig `ToolCallExtensions` or equivalent adapter context for session/execution/ward/auth/store data.
- Preserve MCP tool naming and execution behavior.

**Done when:** Rig executes zbot tools under existing actor policy and result semantics.

### T7: Rig stream and hook events map into existing runtime events

**Depends on:** T4-T6

**Touches:** `runtime/agent-runtime/src/rig_adapter/*`, `runtime/agent-runtime/src/types/*`, `runtime/agent-runtime/src/executor.rs`, `gateway/gateway-execution/src/events.rs`

**Tests:**
- TDD: Rig text/thinking/tool/final/error outputs map to current `StreamEvent` variants in stable order. Verifies AC5, AC15.
- TDD: before-tool, after-tool, transform-context, request override, and tool-result rewrite compatibility behavior is preserved. Verifies AC5, AC11, AC13.
- Goal-based integration: `StreamEvent` -> `GatewayEvent` conversion tests pass unchanged or with intentional additions only. Verifies AC15.

**Approach:**
- Implement the stream adapter from Rig `AgentRunner::stream` output into `StreamEvent`.
- Implement hook compatibility using Rig's hook/flow model.
- Keep current middleware/context control behavior until T9 adapts it deliberately.

**Done when:** gateway-visible streaming behavior is generated from Rig-backed execution through existing event conversion.

### T8: Delegation, continuation, stop, context state, and handoff stay gateway-owned

**Depends on:** T7

**Touches:** `gateway/gateway-execution/src/delegation/*`, `gateway/gateway-execution/src/runner/*`, `gateway/gateway-execution/src/invoke/*`, `runtime/agent-runtime/src/steering*`, `runtime/agent-runtime/src/executor.rs`

**Tests:**
- Goal-based integration: delegated execution persists the parent callback before completion and publishes `SessionContinuationReady` only after the last delegation finishes. Verifies AC6.
- Goal-based integration: stop/cancel does not leave orphaned delegations or a running Rig task. Verifies AC6, AC14.
- TDD: context state events continue to persist resumable runtime state. Verifies AC5, AC6.
- TDD integration: `list_session_agents` and `handoff_to_agent` remain current-session scoped and actor-policy gated. Verifies AC6, AC7.
- TDD: `delegate_to_agent` mode argument and inference still populate `DelegationRequest` and `DelegationContext` without a DB schema change. Verifies AC21.
- TDD integration: child executor initial state exposes delegation mode through the Rig-backed path. Verifies AC21.
- TDD integration: mode-specific executor rules still cover `DirectArtifact`, `WardHygiene`, `WardBackedBuild`, and `StepExecutor` behavior. Verifies AC21.

**Approach:**
- Keep delegation registry, delegation mode handling, continuation watcher, lifecycle, event bus, steering, and handoff validation in gateway-execution/runtime services.
- Adapt Rig pause/termination/error behavior into existing executor result paths.
- Preserve steering and context state export semantics.

**Done when:** existing delegation/continuation/handoff behavior works through the Rig-backed facade.

### T9: Memory and compaction adapt without replacing stores

**Depends on:** T7, T8

**Touches:** `runtime/agent-runtime/src/middleware/*`, `runtime/agent-runtime/src/context_management.rs`, `gateway/gateway-execution/src/invoke/executor.rs`, `stores/*`, `services/*`, `gateway/gateway-memory/*`

**Tests:**
- TDD: runtime-context-control invariants still hold for middleware ordering, tool-pair preservation, plan-block protection, last-resort summarization, prompt-cache compatibility, and durable memory boundaries. Verifies AC13.
- Goal-based integration: memory facts and knowledge graph writes still use existing stores and services. Verifies AC12.
- Goal-based: no durable semantic memory writes are redirected into `conversations.db`. Verifies AC12.

**Approach:**
- Implement Rig `ConversationMemory`/compactor adapters over existing store/service APIs only where useful.
- Keep live context control in AgentZero runtime unless this spec and `runtime-context-control` are both updated.
- Preserve post-run distillation, ward artifact indexing, memory fact, and knowledge graph flows.

**Done when:** Rig memory integration is an adapter over AgentZero persistence, and current context-control invariants still pass.

### T10: Inbound gateway and full wire-event parity pass

**Depends on:** T7-T9

**Touches:** `gateway/src/websocket/*`, `gateway/gateway-ws-protocol/src/*`, `gateway/gateway-events/src/*`, `gateway/gateway-execution/src/events.rs`, `gateway/tests/*`, `apps/ui/src/services/transport/*`

**Tests:**
- Goal-based integration: REST invoke/session payload tests pass through the Rig-backed executor. Verifies AC14.
- Goal-based integration: `ClientMessage` controls and direct responses cover `Connected`, `InvokeAccepted`, `Pong`, `Subscribed`, `Unsubscribed`, `SubscriptionError`, `SessionPaused`, `SessionResumed`, `SessionCancelled`, `SessionEnded`, and direct `Error`. Verifies AC14.
- Goal-based integration: event conversion tests cover `AgentStarted`, `AgentCompleted`, `AgentStopped`, `Token`, `Thinking`, `ToolCall`, `ToolResult`, `TurnComplete`, `Error`, `Iteration`, `ContinuationPrompt`, `DelegationStarted`, `DelegationCompleted`, `Heartbeat`, `MessageAdded`, `TokenUsage`, `WardChanged`, `IterationsExtended`, `PlanUpdate`, `IntentAnalysisStarted`, `IntentAnalysisComplete`, `IntentAnalysisSkipped`, `RecallTrace`, and `SessionTitleChanged`; `SessionContinuationReady` remains internal-only. Verifies AC15.

**Approach:**
- Extend existing websocket/event mapping tests to cover direct and event-derived server messages.
- Keep UI reducers unchanged unless this spec is updated.
- Use current protocol docs as the checklist for non-deferred variants.

**Done when:** inbound and outbound gateway contracts are proven stable at the wire boundary.

### T11: Current DB parity and synthetic E2E pass on Rig-backed execution

**Depends on:** T1, T7-T10

**Touches:** `runtime/agent-runtime/tests/*`, `gateway/gateway-execution/tests/*`, `apps/cli/tests/*`, optional `scripts/*`

**Tests:**
- Goal-based E2E: compare Rig-backed output against the old-engine signature artifacts on deterministic dimensions: committed coarse buckets and sanitized event-path shapes for CI-safe checks, plus exact local event order, role sequence, tool-call/result pairing, delegation/continuation transitions, lifecycle states, and schema-shape hashes when the gitignored local DB signature is available. Verifies AC1, AC15.
- Goal-based E2E: sanitized synthetic cases cover simple chat, tool call/result, delegation/continuation, error, and stop/cancel. Verifies AC1, AC5, AC6, AC14, AC15.
- Goal-based: any skipped synthetic fixture has an explicit spec/plan changelog entry explaining why the source behavior does not exist. Verifies AC1.

**Approach:**
- Run the parity harness against `ZBOT_PARITY_DB`.
- Use synthetic fixtures for committed CI-safe cases.
- Report differences at the event/session boundary rather than internal Rig state.

**Done when:** parity gates prove the Rig-backed path preserves product-visible behavior.

### T12: Persistence crates are rehomed away from zbot-stores

**Depends on:** T9, T11

**Touches:** `Cargo.toml`, `stores/**/*`, `runtime/**/*`, `gateway/**/*`, `services/**/*`, `apps/**/*`

**Tests:**
- Goal-based: `cargo metadata` shows no workspace package named `zero-stores*`. Verifies AC16.
- Goal-based: `rg "zero_stores|zero-stores" Cargo.toml stores runtime gateway services apps` has no active production references except migration history explicitly allowed by the spec. Verifies AC16.
- Goal-based: store conformance tests pass against the renamed traits/domain/SQLite crates. Verifies AC16.
- Goal-based integration: memory, knowledge graph, conversation, episode, procedure, belief, compaction, and vector-index tests pass without schema changes. Verifies AC12, AC16.

**Approach:**
- Rename/rehome `zero-stores-domain`, `zero-stores-traits`, `zero-stores`, `zero-stores-sqlite`, and conformance crates to non-zero names.
- Preserve public trait/domain semantics during the rename.
- Update dependent imports and manifests mechanically, then run conformance gates.

**Done when:** persistence keeps behavior and schema compatibility without active `zero-stores*` names.

### T13: Retire active zero framework crates and imports

**Depends on:** T11, T12

**Touches:** `Cargo.toml`, `framework/**/*`, `runtime/**/*`, `gateway/**/*`, `services/**/*`, `stores/**/*`, `apps/**/*`

**Tests:**
- Goal-based: exact-name searches over active production source/manifests find no legacy crate package or Rust import names: `zero-core`, `zero-agent`, `zero-llm`, `zero-tool`, `zero-mcp`, `zero-session`, `zero-prompt`, `zero-middleware`, `zero-app`, `zero_core`, `zero_agent`, `zero_llm`, `zero_tool`, `zero_mcp`, `zero_session`, `zero_prompt`, `zero_middleware`, or `zero_app`. Verifies AC17.
- Goal-based: `cargo metadata` shows no workspace package named `zero-*`. Verifies AC17.
- Goal-based: `cargo check --workspace` passes. Verifies AC17.

**Approach:**
- Remove or rename framework crates once no active dependency remains.
- Remove compatibility shims and re-exports.
- Update imports, manifests, package metadata, and crate docs.
- Keep only historical references in migration specs/research if needed for audit context.

**Done when:** no active `zero-*` framework code lives in the workspace.

### T14: Active docs describe the Rig-backed architecture

**Depends on:** T12, T13

**Touches:** `AGENTS.md`, `CLAUDE.md`, `framework/AGENTS.md`, `runtime/AGENTS.md`, `stores/AGENTS.md`, `memory-bank/**/*.md`, `docs/**/*.md`, `README.md`

**Tests:**
- Goal-based: active architecture docs no longer present `zero-*` or `zero-stores*` as the live dependency order, framework, or persistence architecture. Verifies AC18.
- Goal-based: docs name the Rig-backed engine, retained gateway/UI contracts, retained memory/knowledge ownership, config mapping, and renamed persistence crates. Verifies AC18.

**Approach:**
- Update workspace layout and dependency-order docs.
- Update memory-bank component docs for execution loop, websocket events, memory boundary, persistence, and runtime architecture.
- Keep research/spec references only as historical migration context.

**Done when:** a new contributor would read active docs and understand Rig is the engine under zbot and persistence no longer uses `zero-*` names.

### T15: Final gates and fresh database manual smoke accepted

**Depends on:** T1-T14

**Touches:** `docs/specs/rig-engine-migration/plan.md`, test docs/scripts as needed

**Tests:**
- Goal-based: current-DB signature parity and mandatory synthetic E2E fixtures are rerun after T12/T13 cleanup and remain green. Verifies AC1, AC14, AC15.
- Goal-based: targeted runtime, gateway-execution, delegation mode, gateway websocket/event, tool, MCP OAuth, connector, store conformance, and store integration tests pass after cleanup. Verifies AC19, AC21.
- Manual QA: user-created fresh database can run chat, stream tokens, execute a first-party tool, complete delegated continuation, and reload the session. Verifies AC20.

**Approach:**
- Run final automated gates.
- Document the fresh DB manual smoke steps and expected observations.
- Record any parity gaps as spec updates or blocking defects.

**Done when:** the migration is mechanically verified and the user accepts the fresh database manual smoke gate as complete.

### T16: Retire prompt-JSON intent analysis

**Depends on:** T7, T15

**Touches:** `runtime/agent-runtime/src/llm/*`, `runtime/agent-runtime/src/rig_adapter/*`, `gateway/gateway-execution/src/middleware/intent_analysis.rs`, `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`, `gateway/gateway-execution/tests/*intent*`, `docs/specs/rig-engine-migration/*`, vault `config/intent_analysis_prompt.md`

**Tests:**
- TDD: runtime OpenAI-compatible request bodies include `response_format: {type: "json_schema"}` when Rig supplies an output schema, and Rig `CompletionRequest.output_schema` reaches `LlmClient::chat_with_schema`.
- TDD: gateway intent analysis accepts valid typed structured output and rejects malformed output through the structured path.
- Goal-based: intent injection tests still prove graph/simple and warm-ward behavior without changing gateway/UI contracts.

**Approach:**
- Add an `LlmClient::chat_with_schema` path and thread it through retry, rate-limit, throttle, non-streaming, OpenAI-compatible transport, and the Rig completion-model adapter.
- Expose a small `agent_runtime::rig_adapter::prompt_typed` helper so gateway-execution can use Rig typed prompts without importing Rig directly.
- Replace the old prompt-JSON parse path in `analyze_intent` with Rig typed structured output; remove markdown-fence stripping and old parse-error wording.
- Tighten `ward_recommendation.action` and `execution_strategy.approach` to serde/schema-backed enums while preserving JSON strings (`use_existing`, `create_new`, `simple`, `graph`).
- Remove the JSON skeleton from the default and live vault intent prompts; prompts now describe decision semantics while runtime owns the schema.

**Done when:** intent analysis uses schema-backed typed output end to end, old prompt-authored JSON parsing is gone, and focused runtime/gateway tests pass.

## Rollout

- **Delivery:** staged internal migration behind the existing `AgentExecutor` facade, then cut over when parity passes. The rollback during implementation is returning the facade to the pre-Rig engine path until T13 removes it.
- **Infrastructure:** no new persistent infrastructure is planned. The parity DB snapshot is external local test data, not repo data.
- **External-system integration:** Rig dependency is pinned in T2; provider/API behavior preserves current OpenAI-compatible settings first.
- **Deployment sequencing:** capture parity baseline, pin Rig, add adapter path, prove runtime/gateway behavior, prove DB/synthetic parity, rehome stores, remove `zero-*`, update docs, then run fresh database manual smoke.

## Risks

- Streaming event order may shift when adapting Rig stream items, breaking UI reducers even if final responses match.
- Tool authorization could become soft if active-tool request overrides are mistaken for executable permission checks.
- Provider behavior may differ between Rig-native clients and current OpenAI-compatible clients.
- Tool result handling could leak hidden context or collapse raw/context/UI result distinctions.
- Current DB parity may expose private data handling concerns; synthetic fixtures must be sanitized.
- Removing all active `zero-*` references is broad and includes persistence crate renames that may touch crates indirectly related to the execution engine.
- Long-lived compatibility shims could leave two frameworks in place unless cleanup is enforced as an acceptance criterion.

## Changelog

- 2026-06-27: initial plan.
- 2026-06-27: tightened constraints, parity signatures, inbound protocol/event coverage, MCP/skill/connector coverage, `zero-stores*` rehome, compaction invariants, and Rig dependency pinning after spec review.
- 2026-06-27: implemented the T1 parity baseline harness: coarse committed DB-derived signature artifact, exact gitignored local DB signature, old-engine event-path signature capture, synthetic fixture generator, and gitignored local provenance manifest.
- 2026-06-27: selected Rig root crate `rig` version `0.39.0` from `https://github.com/0xplaygrounds/rig` pinned to revision `6b1991bfb246411dd75839c8611e801a2309d33c`; added it only to `agent-runtime` with default features disabled, aligned workspace `futures`/`indexmap` minima to Rig's pinned transitive requirements, noted the side-by-side `reqwest 0.13` pulled by `rig-core` with no Rig provider/vector-store features enabled, introduced the `AgentEngine` runtime facade for the current executor, moved gateway root/delegation/continuation execution streams to the boxed facade boundary, and wired `cargo deny check` plus the Rig boundary verifier as supply-chain and isolation gates.
- 2026-06-27: tightened the Rig isolation gate so source imports are allowed only in `runtime/agent-runtime/src/rig_adapter.rs` or a future `rig_adapter/` module, added grouped/aliased provider import detection, and made the Rig pin test compare the declared adapter pin against the actual `agent-runtime` manifest and workspace lockfile.
- 2026-06-27: added the first T4 mapping surface: `LlmConfig` now carries optional provider request params, `agent-runtime` owns neutral Rig-facing agent/model config types, and `gateway-execution` resolves existing agent/provider settings into that config on `ExecutorConfig` without changing live execution.
- 2026-06-27: hardened the T4 mapping after security review: Rig-facing config no longer derives secret-visible serde/debug output, debug output redacts API keys/provider params/instructions, provider params reject reserved request-control keys, and framework-owned `thinking` is added after validation.
- 2026-06-27: preserved `maxInputTokens` inheritance semantics for T4: loaded agents now track whether the field was absent, provider/model context limits apply only to absent values, and an explicit `200000` override remains valid.
- 2026-06-27: tightened that inheritance fix after security review: default-agent seeding now preserves absence, the agent API exposes `maxInputTokensExplicit`, and PUT round-trips no longer materialize inherited defaults as explicit overrides unless the request says so.
- 2026-06-27: extended max-input absence tracking to orchestrator settings so synthesized root and ward agents inherit provider/model limits unless Advanced settings explicitly include `maxInputTokens`.
- 2026-06-27: extended max-input absence tracking to the `create_agent` tool so tool-created agents omit `maxInputTokens` unless the caller supplies it, while explicit `200000` remains preserved.
- 2026-06-27: hardened `create_agent` after security review by rejecting non-kebab-case/path-like agent IDs before filesystem writes and removing the schema default that nudged models to materialize `maxInputTokens`.
- 2026-06-27: further hardened `create_agent` to reject existing target agent directories/symlinks and use non-overwriting file creation for generated config and instructions.
- 2026-06-27: applied the same no-clobber agent creation policy to `AgentService::create` and blocked rename-on-update into an existing agent directory, covering HTTP/API agent creation in addition to tool creation.
- 2026-06-27: reserved `root` and `orchestrator` agent IDs in both tool and service creation paths so persisted agents cannot shadow synthesized system agents.
- 2026-06-27: changed root loading to always synthesize from orchestrator settings and ignore any on-disk `agents/root`, closing root shadowing even if files are created outside the agent APIs.
- 2026-06-27: extended reserved/path validation to agent service read/update/delete/list and rejected delegated `root`/`orchestrator` specialist loads, closing non-create routes to reserved agent folders.
- 2026-06-27: grounded the Rig 0.39.0 (rev `6b1991bf`) API contract from the pinned source at `~/.cargo/git/checkouts/rig-e2b493b9bed14b53/6b1991b` before authoring adapter code (contract-acquisition gate). Decided on **Path A** for the engine: implement a Rig `CompletionModel` adapter over AgentZero's existing `LlmClient` (OpenAI-compatible client + retry + rate-limiter stack), per the T4 approach, so Rig owns the agent loop/tool dispatch/hooks/streaming while AgentZero keeps owning the HTTP transport and rate limiting. Realized this needs **no Rig feature changes**: `CompletionModel`, `Tool`/`ToolDyn`/`Toolset`, `AgentBuilder`/`Agent`/`AgentRunner`, streaming, and the single-method `AgentHook::on_event -> Flow` hook model are all feature-unconditional in `rig-core`; only `reqwest`/`rustls`/`derive`/`rmcp` are gated and none are used, so rig's `reqwest 0.13` stays out of `agent-runtime` and the boundary gate is preserved. Cited contract slice (verifiable against the pinned source): `ToolDyn` requires `name`, `definition(prompt)->ToolDefinition`, `call(args:String)`, `call_with_extensions(args,&ToolCallExtensions)->WasmBoxedFuture<Result<String,ToolError>>`; `ToolCallExtensions` is a typed map (`insert`/`get`/`require`, `Clone+Send+Sync+'static` values) threaded per-request via `AgentRunner::tool_extensions`/`StreamingPromptRequest::tool_extensions`; the agent stream yields `MultiTurnStreamItem::{StreamAssistantItem(StreamedAssistantContent), StreamUserItem(ToolResult), CompletionCall(Usage), FinalResponse}`; `StepEvent::{TextDelta, ToolCall, ToolResult, CompletionResponse, StreamResponseFinish}` steer via `Flow::{RewriteArgs, RewriteResult, OverrideRequest, Terminate}`; `WasmBoxedFuture<'a,T> = Pin<Box<dyn Future<Output=T>+Send+'a>>` (native) at `rig::wasm_compat`.
- 2026-06-27: implemented **T6** (tool bridge into Rig) in `runtime/agent-runtime/src/rig_adapter/tool.rs`: `RigToolAdapter` implements `rig::tool::ToolDyn` over an existing `zero_core::Tool`, mapping `definition()` to name/description/parameters-schema, and dispatching `call_with_extensions` into `zero_core::Tool::execute`. Hidden runtime context rides Rig's `ToolCallExtensions` as a shared `Arc<ToolContext>` (`SharedToolContext` alias), extracted into an owned value before the async block so no borrow crosses the await boundary and the no-extensions `call` path needs no temporary. Result rendering mirrors rig's `serialize_tool_output` (string verbatim, else JSON); the richer `context_result`/offload rewriting is deferred to the engine's `Flow::RewriteResult` hook in T7. Seven TDD tests prove schema mapping, hidden-context-flows-without-leaking-into-args-or-schema (AC7/AC10), null-arg normalization, model-visible result shaping (AC11), shared-context state persistence across calls (load_skill semantics), degraded-empty fallback (now `tracing::warn!`-logged), and empty-schema fallback. Workspace check, full `agent-runtime`/`gateway-execution` suites, and the Rig boundary gate remain green.
- 2026-06-27: T6 adversarial review (security lens skipped per user). Two Concerns applied: (1) removed `FunctionCallId` threading from the bridge — rig builds one `ToolCallExtensions` per request so it cannot represent a distinct call id per tool in a turn, and writing it onto the shared `ToolContext`'s single `function_call_id` field would race under `tool_concurrency > 1`; call-id fidelity is now a T7 engine-concurrency concern (resolve via the `StepEvent::ToolCall` hook + a concurrency decision there); (2) strengthened `hidden_context_flows_via_extensions_not_args` to assert against what the tool observed (`call.args`), not the caller-side args string (which was tautological). Nit on silent degradation applied (`tracing::warn!` on the no-context path). Two nits judged structural/parity and left: the `definition()` clone+box is forced by the `WasmBoxedFuture` return type; `Box::new(e)` type-erases `ZeroError` but matches rig's own blanket impl and the engine handles errors before they cross into rig.
- 2026-06-27: started **T7** — the Rig execution cutover. Grounded the remaining contract (Rig's `CompletionModel`/`StreamingPrompt`/`MultiTurnStreamItem`/`StreamedAssistantContent`/`StreamingError`/`AgentBuilder.build`/`GetTokenUsage`/`RawStreamingChoice`/`StreamingCompletionResponse::stream`) from the pinned source; key API facts: `StreamingPrompt` and `StreamedAssistantContent` live in `rig::streaming`, `MultiTurnStreamItem`/`StreamingError` in `rig::agent`, `()` already implements `GetTokenUsage` so a stub model needs no custom response type, `AgentBuilder::build` is sync, and awaiting `StreamingPromptRequest` yields the agent-level stream directly (`send()` is private — do not call it). Implemented `runtime/agent-runtime/src/rig_adapter/engine.rs`: `RigAgentEngine<M: CompletionModel>` implementing the gateway-facing `AgentEngine` facade — builds the Rig agent once from `RigAgentConfig` + model + bridged `ToolDyn` set, threads the shared `Arc<ToolContext>` per run via `ToolCallExtensions`, drives `stream_prompt`, and maps `MultiTurnStreamItem` onto `StreamEvent` (text→`Token`, tool-call→`ToolCallStart`, finalization→`Done`), with cooperative stop and streaming-error→`ExecutorError` mapping. Three TDD tests (stub model) prove stable token ordering + finalization, empty-model finalization, and cooperative stop. Workspace, full `agent-runtime` suite, and boundary gate green. Remaining T7: LlmClient-backed `CompletionModel` (T7a, the provider bridge that lets Rig drive the real OpenAI-compatible stack), full mapping (reasoning→`Reasoning`, `StreamedUserContent::ToolResult`→`ToolResult` incl. raw/context distinction, `ChatMessage`→`Message` history + `stream_chat`, `TokenUpdate`), and `AgentHook` surfacing (T7c, which also restores per-call `function_call_id`).
- 2026-06-27: completed the T7 provider bridge and full mapping. Added `runtime/agent-runtime/src/rig_adapter/model.rs`: `LlmCompletionModel` implements Rig's `CompletionModel` over AgentZero's `LlmClient` — `convert_messages`/`convert_tools` translate a Rig `CompletionRequest` into AgentZero `Vec<ChatMessage>` + OpenAI tools payload, and `stream()` drives `LlmClient::chat_stream`, bridging its callback chunks onto Rig's stream via `futures::mpsc::unbounded` (sync send from inside the async `chat_stream`, where `tokio::mpsc::blocking_send` would panic). Text/reasoning stream live; tool calls emit complete when `chat_stream` resolves; errors surface as `CompletionError::ProviderError`. Design note: tool-call argument fragments are not re-accumulated (the authoritative complete calls come back in `ChatResponse`), avoiding duplicate provider parsing. `engine.rs` now drives `stream_chat` with `ChatMessage`→`Message` history conversion, `tool_concurrency(1)` (keeps the shared `ToolContext` race-free pending T7c's per-call carrier), and maps reasoning→`Reasoning` and `StreamedUserContent::ToolResult`→`ToolResult`. Tests: 3 bridge tests + an end-to-end test driving the real `LlmCompletionModel` through `RigAgentEngine` with a stub `LlmClient` (full `LlmClient → Rig agent loop → StreamEvent` chain, no network). 21 rig_adapter tests pass; workspace, full `agent-runtime`/`gateway-execution` suites, and boundary gate green; light adversarial review (correctness only, security skipped) returned `Clean — ready to commit.` Deferred: token-usage through the bridge (`TokenUpdate`), raw/context_result distinction + tool-role history, and `AgentHook` before/after-tool + result-rewrite surfacing (T7c, restores per-call `function_call_id`).
- 2026-06-27: completed **T7c** — AgentZero execution-hook surfacing. Added `RigExecutionHook<M>: rig::agent::AgentHook<M>` to `engine.rs`, threaded into the agent via a new `RigAgentEngine::with_tool_hooks(config, model, tools, shared_context, before, after)` constructor (the existing `new`/`with_max_turns` attach a no-hook instance). The hook: on `StepEvent::ToolCall` sets the shared `ToolContext`'s `function_call_id` from the call id (resolving the T6 concurrency concern — `tool_concurrency(1)` keeps it race-free) and maps `before_tool_call`'s `ToolCallDecision::Block` to `Flow::skip` (reason returned to the model as the tool result); on `StepEvent::ToolResult` maps `after_tool_call`'s optional replacement to `Flow::rewrite_result` (model-visible only; the real result still ran), calling it with `succeeded=true` to match the legacy executor's success path. Two TDD tests prove a `Block` prevents tool execution (call count 0, run still finalizes) and an `Allow` runs the tool once with the `function_call_id` correctly threaded from the hook into the bridged tool's context. 6 engine tests pass; workspace, full `agent-runtime`/`gateway-execution` suites, and boundary gate green. T7 (bridge + mapping + hooks) is now complete; remaining fidelity items only: token-usage through the bridge (`TokenUpdate`) and the raw/context_result distinction + tool-role history.
- 2026-06-27: completed **T8** — verified gateway-owned delegation/continuation/handoff/delegation-modes are preserved through the `AgentEngine` facade after T3/T7, and proved the Rig path carries child-executor delegation mode. Ran the existing AC6/AC21 suites green: `continuation_watcher_tests` (3) and `delegation_dispatcher_tests` (5) (both `#![cfg(feature = "test-stubs")]`-gated), `e2e_ward_pipeline_tests` (12, covering the `DirectArtifact`/`WardHygiene`/`WardBackedBuild`/`StepExecutor` mode rules and delegation callback ordering), and `ward_agent_spawn_tests` (4). Added one new test — `delegation_mode_flows_to_tool_through_rig_path` — proving the AC21 invariant that a child executor's seeded delegation mode (`app:delegation_mode`) reaches a bridged tool through `RigAgentEngine`'s `SharedToolContext`, the mechanism that lets the Rig path assume child-executor duties once it becomes default. No production code change: T8 is a preservation/verification gate and the architecture already carries these through the facade. Full `agent-runtime`/`gateway-execution` suites and boundary gate green. Remaining (cutover-gated, not blocking): full end-to-end delegation/continuation exercised through `RigAgentEngine` as the live child engine — wired when the default flips at T11.
- 2026-06-27: completed **T9** — memory/compaction preservation. Verified AC13 (runtime-context-control invariants) and AC12 (durable memory via existing stores) still hold: 113 agent-runtime middleware + `context_management` tests pass (middleware ordering, tool-pair preservation, plan-block protection, last-resort summarization, prompt-cache, token thresholds), and `recall_unified` passes (memory facts + knowledge graph through existing stores). `RigAgentEngine` preserves both ACs by construction — it holds no durable store handle (only an in-process `SharedToolContext`) and receives the gateway-owned conversation tape, so no semantic memory can be redirected into `conversations.db` and live context control stays in AgentZero runtime per the `runtime-context-control` constraint. No speculative Rig `ConversationMemory` adapter was added: the gateway already owns conversation load/persist, so it would be unused. Added one new test — `rig_engine_forwards_history_to_llm_unchanged` — proving the Rig path forwards the full conversation tape (prior user+assistant turns and the current prompt) to the `LlmClient` without silent compaction or loss. Full `agent-runtime`/`gateway-execution` suites and boundary gate green. Cutover-gated follow-up (T11): wire the gateway's middleware pipeline to compact the tape for the Rig path when it becomes the live default.
- 2026-06-27: completed **T10** — inbound gateway + full wire-event parity. Verified AC14 (inbound: REST invoke/session, `ClientMessage` controls, direct `ServerMessage` responses) and AC15 (every event-derived `GatewayEvent → ServerMessage` variant) are stable at the wire boundary: `gateway-ws-protocol` (7), `gateway-events` (8), and `gateway` websocket/REST/invoke suites (12 + 2) all pass. No production change: Rig remains confined to `agent-runtime` (boundary gate clean), so the websocket router, `gateway-ws-protocol`, and `gateway-events` conversion are untouched by T7–T9 and UI reducers are unchanged. The "through the Rig-backed executor" qualifier on REST invoke is cutover-gated (the live default is still `AgentExecutor` behind the facade) and is exercised at T11.
- 2026-06-27: completed **T11** — Rig-backed parity via a test harness, **no live default flip** (per user decision). Added `gateway/gateway-execution/tests/rig_parity_tests.rs`: drives `RigAgentEngine` (through the real `LlmCompletionModel` bridge over a scripted stub `LlmClient`) through the Rig-path-derived synthetic scenarios, runs its runtime `StreamEvent`s through the gateway's `convert_stream_event`, and asserts the resulting `GatewayEvent` sequence matches the old-engine baseline. Four scenarios pass: `simple_chat` (Token + TurnComplete, no tool/error), `tool_call_result` (ToolCall → ToolResult → TurnComplete, in order), `error` (LLM error surfaces as `ExecutorError::LlmError`, matching the legacy executor — the gateway converts the `Err` into the Error event, so no `StreamEvent::Error` is expected from either path), and `stop_cancel` (stop flag halts after the first token and the run finalizes Done → TurnComplete). `delegation_continuation` is gateway-owned (`Delegation*` events come from the delegation registry, not the engine) — verified in T8; gateway-lifecycle events (`AgentStarted`/`AgentCompleted`/`AgentStopped`/`SessionCancelled`) are emitted by the runner around the executor and covered by T10. Boundary gate stays clean (gateway-execution does not depend on the `rig` crate; the harness uses only `agent_runtime`'s public adapter facade). Workspace + boundary green. The Rig path is now proven gateway-event-equivalent to the old engine without being wired as default; the actual cutover (flipping the facade's default executor) remains a separate, explicit step.
- 2026-06-28: **cutover wired** — the Rig-backed engine is now selectable in production behind `ZBOT_ENGINE=rig` (default OFF = legacy `AgentExecutor`). Added `gateway-execution::invoke::executor::select_engine(AgentExecutor) -> BoxedAgentEngine`, called at the three boxing sites (`runner/core.rs`, `delegation/spawn.rs`, `runner/invoke_bootstrap.rs`) that previously did `Box::new(executor)`. When the flag is set, a `RigAgentConfig` is resolved, and **no MCP servers are configured**, it constructs `RigAgentEngine::with_tool_hooks` from the same `LlmClient` (new `AgentExecutor::llm_client()` accessor), the same actor-filtered tool inventory bridged via `RigToolAdapter` (new `tool_registry()` accessor), a `SharedToolContext` built from the config's `agent_id`/`conversation_id`/`skills`/`initial_state`, and the config's before/after-tool hooks. Added `AgentEngine::engine_name()` (`"agent-executor"` default, `"rig"` override) for observability and selector testing. The MCP safety gate (fall back to legacy when `config.mcps` is non-empty) avoids orphaning subprocesses — `McpManager` has no `Drop` cleanup, so the Rig path does not yet bridge MCP. Two tests cover the routing (legacy default; rig when enabled + no MCP; legacy fallback when MCP present). Known Rig-path limitations while the flag is on (live A/B validation only): no middleware/compaction, no token-usage events, no mid-session recall/steering hooks, no MCP. Full workspace + boundary + `agent-runtime`/`gateway-execution` suites green.
- 2026-06-28: completed **T13** — retired the active framework layer instead of keeping compatibility shims. Moved the still-live shared primitive surface from `framework/zero-core` into the new `runtime/agent-primitives` crate, rewired active runtime/gateway/store/app manifests and imports to `agent-primitives`, renamed the carried error type from `ZeroError` to `AgentError`, removed stale `zero-app` dependencies, deleted the retired `framework/zero-*` tree, and updated active AGENTS/spec text that still described the old framework as live. Verification: exact legacy package/import search over active source/manifests/lockfile is clean, `cargo metadata` reports no `zero-*` workspace packages, `cargo fmt --all -- --check` passes after formatting, and `cargo check --workspace` passes. Per user direction, no adversarial review or Rig boundary check was run for this pass.
- 2026-06-28: completed **T14** — cleaned active architecture docs after the framework retirement. Updated the root engineering notes, memory-bank architecture/component docs, LLM-client docs, spec index, and `docs/project-graph.html` so active documentation now presents the live `runtime/agent-primitives`, `agent-runtime`, `agent-tools`, `zbot-stores*`, gateway, services, and app crate graph instead of the deleted framework layer. Historical research/spec audit material intentionally remains as migration context. Per user direction, no adversarial review or Rig boundary check was run for this pass.
- 2026-06-28: completed **T15** — final gates passed after cleanup/adversarial fixes (`cargo fmt --all -- --check`, `git diff --check`, active stale-name searches, `cargo check --workspace`, `cargo test -p agent-primitives`, `cargo test -p agent-runtime`, `cargo test -p agent-tools`, `cargo test -p gateway-execution`, `cargo test -p zbot-stores-sqlite`). User accepted the fresh database manual smoke gate as complete; remaining Rig-path fidelity follow-ups stay tracked in `e2e_test_cases.md`.
- 2026-06-28: completed **T16** — retired the old intent-analysis prompt-JSON path in favor of Rig typed structured output while preserving gateway events and persisted field names. Added schema-aware LLM transport plumbing (`chat_with_schema`, OpenAI-compatible `response_format`, Rig `CompletionRequest.output_schema` forwarding), exposed a runtime-boundary `prompt_typed` helper, tightened `action`/`approach` to serde/schema enums, removed markdown-fence parsing and JSON skeleton prompt text, and updated the live vault prompt. Verification: `cargo test -p agent-runtime schema --no-default-features`, `cargo test -p gateway-execution intent_analysis --no-default-features`, `cargo test -p gateway-execution --test intent_analysis_tests --no-default-features`, `cargo test -p gateway-execution --test e2e_ward_pipeline_tests --no-default-features`, `cargo fmt --all -- --check`, and `git diff --check`.
