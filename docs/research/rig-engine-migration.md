# Rig Engine Migration Report

Date: 2026-06-27

Rig checkout: `rig checkout`, commit `6b1991bf`.

## Codemem Evidence Used

Codemem was used for both AgentZero and Rig.

- AgentZero index status: indexed with embeddings and knowledge graph data in `local Codemem index`; current status reported 1,474 files, 23,537 chunks, 20,185 graph nodes, 22,492 graph edges, and 80,950 references.
- Rig index status: project `rig`, 1,146 files, 24,162 chunks, 31,322 entity embeddings, 23,555 graph nodes, and 26,059 graph edges.
- Codemem search covered `zero-core`, `zero-agent`, `agent-runtime`, `gateway-execution`, gateway websocket event flow, Rig `AgentRun`, `AgentRunner`, hooks, tool server, tool extensions, and memory/compaction.
- Codemem impact analysis covered `framework/zero-core`, `framework/zero-agent`, `runtime/agent-runtime`, and `gateway/gateway-execution`.
- A direct `graph_query references_to` pass on crate `lib.rs` roots returned no rows, so the stronger knowledge-graph signal in this report is the named-target impact analysis plus targeted code/doc search. The broad path-root graph query was noisy.

This report is intentionally about replacing the underlying engine while preserving the product contract: gateway, UI events, memory/knowledge stores, current configs, and most gateway execution behavior.

## Executive Answer

If you replace `zero-core` and `zero-agent` with Rig as the underlying agent engine, the main gain is not raw feature count. AgentZero already has substantial orchestration, delegation, memory, tools, and gateway behavior. The gain is that Rig gives you a cleaner, more modern engine boundary for the pieces currently spread across framework traits, runtime executor loops, hooks, middleware, provider plumbing, and tool execution.

The biggest gains are:

1. A concrete agent builder and runner model instead of a small trait-only agent abstraction.
2. A serializable, sans-IO `AgentRun` state machine that can make continuation, pause/resume, and durable orchestration less ad hoc.
3. A unified hook/flow lifecycle model that can replace scattered `before_tool_call`, `after_tool_call`, `transform_context`, middleware, and gateway policy branches over time.
4. Typed tools and dynamic tool server support, while still allowing dynamic tools and MCP tools.
5. Per-call hidden tool context through `ToolCallExtensions`, which maps well to session IDs, actor kind, ward ID, auth scope, trace IDs, and runtime-only policy context.
6. Streaming/non-streaming parity in the agent loop, which reduces drift between test paths and UI paths.
7. Explicit memory and compaction interfaces that can rationalize current context editing and summarization code.

The biggest non-gain is also important: Rig does not replace AgentZero's product orchestration by itself. It does not know your `GatewayEvent` contract, delegation continuation semantics, actor tool policy, memory stores, knowledge graph, ward model, config files, or UI wire protocol. Those should remain AgentZero-owned.

Recommendation: migrate by inserting a Rig-backed engine adapter behind the existing gateway/runtime boundary first. Do not begin by deleting gateway execution or changing UI events.

## Premise And Invariants

The migration should preserve these invariants:

- The UI still talks to gateway over the same HTTP/WebSocket contract.
- `GatewayEvent` and `ServerMessage` semantics remain stable.
- Session, execution, conversation, delegation, continuation, and stop/cancel behavior remain gateway-owned.
- Existing memory and knowledge graph stores remain the source of truth.
- Existing provider, agent, MCP, skill, connector, and tool settings formats remain accepted.
- Gateway actor policy remains enforceable outside prompt text.
- Current config files remain the user-facing control plane.

The engine can change under those invariants:

- `zero_core::Agent` and `zero_agent::*` can be retired or reduced to compatibility shims.
- `agent_runtime::AgentExecutor` can become a facade over Rig.
- Tool exposure can change internally, as long as advertised tool schemas and gateway policy remain stable.
- Compaction can gain a Rig-like memory policy layer, as long as live context survival remains at least as good as today.

## Current AgentZero Shape

### Framework Layer

`zero-core` is a minimal foundational trait crate. Codemem search and local reads show it owns the basic `Agent`, `Tool`, `Toolset`, context, event, capability, permission, and registry abstractions. The workspace dependency map places `zero-core` below `zero-llm`, `zero-tool`, `zero-mcp`, `zero-session`, `zero-prompt`, `zero-middleware`, `zero-agent`, and `zero-app`.

`zero-agent` is higher level but still relatively thin:

- `framework/zero-agent/src/llm_agent.rs` defines `LlmAgent` as `name`, `description`, `Arc<dyn Llm>`, `Arc<dyn Toolset>`, and optional system instruction.
- It builds an LLM request from session history and `ToolDefinition`s, executes returned tool calls, and appends tool responses.
- `framework/zero-agent/src/orchestrator/mod.rs` provides capability-based routing, task graphs, parallel task execution, retries, and execution traces using `CapabilityRegistry`.

The framework layer is replaceable in principle, but its types leak into runtime tools and stores.

### Runtime Layer

`runtime/agent-runtime/src/lib.rs` already describes itself as a modular execution framework with types, LLM abstraction, tools, MCP, middleware, executor, steering, logging, and context management. This means there are effectively two agent-framework layers:

- `framework/*`, which is publishable and trait-oriented.
- `runtime/agent-runtime`, which is the actual product execution engine.

The active executor is `agent_runtime::AgentExecutor`. It owns:

- LLM client streaming.
- Tool registry and MCP manager.
- Tool schema hardening and MCP tool normalization.
- Tool execution, including built-in tools and MCP tools.
- `before_tool_call`, `after_tool_call`, and `transform_context`.
- sequential/parallel tool execution behavior.
- context editing and summarization middleware.
- steering queue injection.
- turn budgets, max turns, stuck detection, and single-action mode.
- `StreamEvent` emission.

This runtime is where most migration risk lives.

### Gateway Execution Layer

`gateway/gateway-execution` is not just a thin caller. Codemem found it converts runtime `StreamEvent` values to `GatewayEvent`, manages agent delegation with callbacks, handles continuation turns, and emits lifecycle events.

Important retained surfaces:

- `gateway/gateway-execution/src/invoke/stream_event_processor.rs` handles artifacts, delegation side effects, and converts `StreamEvent` to `GatewayEvent`.
- `gateway/gateway-execution/src/runner/execution_stream.rs` calls `executor.execute_stream_with_stop_flag(...)`, processes runtime events, accumulates response deltas, persists session messages, triggers recall, completes/crashes/stops execution, and runs post-session distillation/indexing/handoff.
- `gateway/gateway-execution/src/delegation/spawn.rs` emits `DelegationCompleted`, persists parent callbacks before completing delegation, and publishes `SessionContinuationReady`.
- `gateway/src/websocket/handler.rs` routes `GatewayEvent` through subscription scoping to `ServerMessage`.
- `memory-bank/websocket-events.md` documents field transformations from `GatewayEvent` to wire protocol.

This layer should be preserved first. It is the adapter client.

## What Rig Gives You

### 1. Engine Cohesion

Rig's `Agent<M>` is not just a trait. It is a configured runtime object built around a `CompletionModel`, prompt/preamble/static context, tools, dynamic context, output schema/mode, hooks, memory, conversation ID, and default turn limits.

That directly replaces the split between:

- `zero_core::Agent`.
- `zero_agent::LlmAgent`.
- pieces of `zero_llm`.
- pieces of `agent_runtime::ExecutorConfig`.
- parts of `create_executor` wiring.

The gain is less conceptual duplication. Today AgentZero has minimal framework traits plus a separate runtime engine. With Rig, the engine boundary is already a first-class library boundary.

### 2. Serializable Run State

Rig's `AgentRun` is a sans-IO state machine. Codemem found `AgentRunStep` variants such as `CallModel`, `CallTools`, and `Done`, and `RunState` carrying pending tool calls for resume.

That matters for AgentZero because your gateway already has:

- delegation pause/resume.
- `SessionContinuationReady`.
- stop/cancel handling.
- steering.
- wait-agent/delegation callbacks.
- execution-state persistence.
- WebSocket reconnect and session replay requirements.

Today those concerns are distributed between gateway state, runtime loop variables, conversation history, tool result conventions, and events. A serializable run state gives you a cleaner place to persist "what should happen next" without treating the transcript as the only state machine.

This is the highest-value Rig idea for zbot if the goal is rationalizing the engine while preserving the product contract.

### 3. Hook And Flow Rationalization

Rig has a single hook shape: `AgentHook::on_event(StepEvent) -> Flow`. The changelog and code show one hook stack covering model calls, responses, tool calls, tool results, invalid tool calls, streaming deltas, request overrides, tool argument rewrites, tool result rewrites, skip/terminate behavior, and fail-closed semantics.

AgentZero currently splits equivalent policy across:

- executor config hooks.
- middleware.
- gateway actor-kind tool filtering.
- tool implementations.
- stream event processing.
- context editing and summarization.
- delegation side effects.

Rig gives you a vocabulary for rationalizing this:

- `CompletionCall`: transform prompt/context, active tools, sampling, provider params.
- `ToolCall`: approve, deny, rewrite arguments, attach policy.
- `ToolResult`: redact, truncate, summarize, emit side effects.
- `TextDelta` and `ToolCallDelta`: observe streaming without changing final history.
- `InvalidToolCall`: repair/retry/fail policy.
- `Terminate`: cleanly stop a run.

You should not move all policy into prompt hooks. Gateway actor policy must remain a hard boundary. But a Rig-style lifecycle enum can make the policy flow explicit and testable.

### 4. Typed Tools Plus Dynamic Tools

AgentZero tools are JSON-first through `zero_core::Tool`: name, description, parameter schema, response schema, permissions, validation, and `execute(ctx, Value)`. This is flexible for gateway and external tools, but tool authoring and return contracts are weak.

Rig gives you:

- typed tool arguments/output/errors.
- dynamic dispatch bridges.
- `ToolServerHandle`.
- dynamic tools.
- MCP tool integration.
- `Agent` as a `Tool`.
- hidden `ToolCallExtensions` propagated into tools and subagents.

For zbot, the important gain is not replacing JSON schemas at the UI boundary. It is having a better internal authoring model and a safer runtime context channel.

Likely result:

- First-party tools become Rig tools or Rig-wrapped tools.
- MCP tools use Rig's MCP surface or stay behind an adapter.
- Existing JSON schemas remain what the model and UI/gateway inspect.
- Gateway actor policies still choose the executable tool set.

### 5. Hidden Runtime Context For Tools

Rig's `ToolCallExtensions` lets callers attach runtime-only values to tool calls without exposing them to the model. The changelog explicitly calls out auth tokens, session IDs, A2A IDs, and conversation state.

This maps cleanly to AgentZero:

- `session_id`
- `execution_id`
- `conversation_id`
- `agent_id`
- `ward_id`
- `RuntimeActorKind`
- `FileSystemContext`
- memory and knowledge graph handles
- bridge/connector auth scopes
- UI approval context
- trace IDs

Today much of this is carried through custom `ToolContext`, gateway builder fields, shared runtime context, and tool-specific assumptions. Rig's extension channel would let tool execution keep those values strongly typed and hidden from model-visible arguments.

### 6. Streaming Parity

Rig's current agent runner shares construction, tool execution, hooks, and memory between blocking and streaming paths. Streaming has extra delta events, but the medium-independent hook sequence is intended to match.

AgentZero's UI depends on streaming. Tests and non-streaming paths can drift from streaming behavior when the loop is hand-coded. A Rig-backed adapter could reduce that drift if all zbot execution uses Rig's streaming driver and maps Rig stream items into existing `StreamEvent` variants.

### 7. Compaction As A First-Class Interface

Rig's compaction model is centered on conversation memory:

- `ConversationMemory` loads ordered messages before a run and appends successful turns afterward.
- memory policies choose what to keep.
- demotion hooks observe evicted messages.
- compactors convert evicted messages plus carry-over into a bounded message-like summary.
- the resulting prompt shape is roughly `[summary, ...recent_window]`.

AgentZero's current compaction is live middleware:

- context editing clears old tool results and can cascade unload resources.
- summarization optionally calls an LLM and inserts a system summary.
- gateway orders context editing, plan-block reinsertion, and summarization.

Rig gives cleaner interfaces. AgentZero currently gives more aggressive survival behavior. The best migration is to put a Rig-like compactor interface behind AgentZero's existing live middleware first, then consider a durable `ConversationMemory` adapter over `zbot-stores`.

## What You Probably Lose Or Must Rebuild

### 1. Existing Event Semantics Are Not Native To Rig

Rig stream items are not AgentZero `StreamEvent`s. You will need an adapter that emits the existing runtime events:

- token/thinking deltas.
- tool call start/end.
- tool result with raw result and context result.
- action/delegation/respond events.
- done/stopped/error events.
- context state events.
- session title and artifact side effects.

This adapter is the core compatibility layer.

### 2. Gateway Actor Policy Must Stay Yours

Rig can restrict active tools per turn, but zbot's safety boundary is stronger:

- Root has orchestration/session tools.
- Delegated executor gets implementation tools but not orchestration tools.
- Delegated reviewer gets read-only tools.
- Ward agent gets broader powers.

Codemem impact found tests around `RuntimeActorKind`, `ToolCapability`, `ExecutorBuilder`, and actor tool access. Keep those tests. Rig active-tools request overrides are useful, but they should sit after gateway executable-tool filtering, not replace it.

### 3. Provider Flexibility Needs A Decision

AgentZero currently leans on OpenAI-compatible provider settings and runtime config. Rig has provider-specific crates and capability modeling.

You have two choices:

- Use Rig's native providers where possible and write compatibility for current provider config.
- Implement a custom AgentZero `CompletionModel` over the existing `agent_runtime::OpenAiClient`/provider config stack.

The second path preserves user config and lowers migration risk. The first path may produce a cleaner long-term provider layer but risks breaking OpenAI-compatible custom endpoints and gateway settings.

### 4. Tool Result Shaping Must Be Preserved

AgentZero does important tool-result shaping:

- large result offload.
- prompt-safe context result.
- raw result versus model-visible result.
- action extraction from tool context.
- tool errors as model-visible messages.
- session message persistence.

Rig has tool result rewrite hooks, but zbot still needs the distinction between persisted raw output, UI output, and model-context output. Preserve that distinction in the adapter.

### 5. Memory Semantics Are Different

Rig memory appends successful turns after completion. AgentZero's gateway/runtime updates state and context during execution, including working memory, delegation state, recall, distillation, session handoff, and knowledge graph indexing.

You can use Rig memory interfaces, but the product memory/knowledge system should remain authoritative.

## Layer Impact Map

Codemem impact analysis shows this is not a two-crate delete. It is a layered migration.

| Layer | Migration Role | Expected Impact | Notes |
| --- | --- | --- | --- |
| `framework/zero-core` | Retire, shrink, or compatibility shim | High in framework/runtime tools | Many first-party tools import `zero_core::Tool`, `ToolContext`, `FileSystemContext`, `Content`, and errors. |
| `framework/zero-agent` | Replace | Medium | Mostly LLM agent, workflow agents, and orchestrator. Less tied to active gateway path than runtime. |
| `framework/zero-tool` | Replace or adapt | Medium | Current registry and function-tool helpers can become adapters. |
| `framework/zero-llm` | Replace or adapt | Medium | If Rig native providers are used, this shrinks. If current provider stack is preserved, it may become an adapter. |
| `framework/zero-mcp` | Replace or adapt | Medium | Rig has MCP support, but gateway config and OAuth flows must be preserved. |
| `framework/zero-app` | Rewrite | Medium | Prelude crate can re-export new adapter types during transition. |
| `runtime/agent-runtime` | Engine facade over Rig | Very high | The active loop, stream events, hooks, middleware, tools, MCP, steering, and context management live here. |
| `runtime/agent-tools` | Tool bridge | High | Many tools implement `zero_core::Tool`; they need Rig wrappers or direct conversion. |
| `gateway/gateway-execution` | Preserve API, change executor internals | High but controlled | Keep runner/delegation/continuation/event conversion stable. |
| `gateway/gateway-services` | Mostly preserve | Low to medium | Provider/MCP settings and agent config loading feed the engine adapter. |
| `gateway/src/websocket` | Preserve | Low | Should only change if `GatewayEvent` changes, which this plan avoids. |
| `stores/*` | Preserve | Low to medium | Some stores depend on `agent_runtime::ChatMessage` and embedding clients; avoid changing those first. |
| `apps/ui` | Preserve | Low | WebSocket and REST contracts remain stable. |

## Proposed Target Architecture

The first stable architecture should look like this:

```text
UI
  |
  v
gateway HTTP/WS contract
  |
  v
gateway-execution runner/delegation/continuation/state
  |
  v
AgentExecutor facade
  |
  v
RigEngineAdapter
  |-- config mapper: existing providers.json/agent YAML/settings -> Rig AgentBuilder/AgentRunner
  |-- provider adapter: existing OpenAI-compatible stack -> Rig CompletionModel
  |-- tool adapter: existing zbot tools/MCP -> Rig ToolServerHandle
  |-- hook adapter: zbot policy/context hooks -> Rig AgentHook/Flow
  |-- stream adapter: Rig stream items -> agent_runtime::StreamEvent
  |-- memory adapter: zbot-stores/runtime context -> Rig ConversationMemory/compactor where useful
  |
  v
Rig AgentRunner / AgentRun
```

The facade lets you keep `gateway-execution` stable while the internals change.

## Migration Phases

### Phase 0: Freeze The Contract

Before replacing engine code, lock down the contract that must not move.

Add or confirm tests for:

- `StreamEvent` to `GatewayEvent` conversion.
- `GatewayEvent` to `ServerMessage` websocket mapping.
- token streaming and final turn completion.
- tool call start/result/end event ordering.
- delegation started/completed and `SessionContinuationReady`.
- stop/cancel behavior.
- context state event persistence.
- actor-kind tool access.
- session message persistence.
- memory and knowledge graph post-run hooks.

This is the safety rail for the migration.

### Phase 1: Add A Rig Dependency Behind A Feature Or Adapter Crate

Add a small adapter crate or module before changing the runtime:

- `runtime/agent-runtime/src/rig_adapter/*`, or
- a new `runtime/agent-engine-rig` crate.

The adapter should expose the smallest interface needed by `AgentExecutor`:

```rust
trait EngineRun {
    async fn execute_stream(
        &self,
        message: &str,
        history: &[ChatMessage],
        stop: Option<StopSignal>,
        on_event: impl FnMut(StreamEvent),
    ) -> Result<(), ExecutorError>;
}
```

The point is to avoid pushing Rig types into gateway first.

### Phase 2: Provider Adapter

Preserve existing configs first.

Build an AgentZero `CompletionModel` implementation over the current OpenAI-compatible client stack. Map:

- provider ID.
- model.
- base URL.
- API key handling.
- temperature.
- max tokens.
- thinking/reasoning config.
- additional provider params.
- tool schema behavior.
- streaming chunks.
- token usage.

Only after this works should you decide whether to adopt Rig-native providers.

### Phase 3: Tool Adapter

Build a bridge from current tools to Rig tools.

Required behavior:

- convert current `zero_core::Tool` schema into Rig/provider tool definitions.
- pass hidden runtime context through `ToolCallExtensions`.
- preserve actor-kind executable tool filtering before tools reach Rig.
- preserve raw result versus model-visible `context_result`.
- preserve `EventActions` extraction.
- preserve MCP tool naming and normalization.
- preserve replay intercepts if still required.
- preserve large result offload and prompt-safe truncation.

This phase is where tool exposure changes, but the UI/gateway contract should not.

### Phase 4: Stream Adapter

Map Rig stream items and hook observations into existing `StreamEvent`.

Minimum event mapping:

- text delta -> `StreamEvent::Token`.
- reasoning delta, if available -> existing thinking/reasoning event.
- tool call delta/start -> `ToolCallStart` once enough data exists.
- tool result -> `ToolResult`.
- final response -> `Done`.
- errors -> existing executor error path.
- usage -> existing token accounting path.

The adapter should be deterministic about event ordering. Gateway event conversion and UI reducers assume stable order.

### Phase 5: Hook/Policy Adapter

Start by preserving current behavior:

- `transform_context` maps to a pre-completion hook or pre-run history transform.
- `before_tool_call` maps to a `ToolCall` hook.
- `after_tool_call` maps to a `ToolResult` hook.
- actor policy remains outside Rig as executable tool filtering.
- tool result redaction/truncation remains the final model-visible result.

Then consolidate:

- request override for phase-specific active tools.
- tool arg rewrite for path/ward scoping.
- tool result rewrite for redaction/offload.
- fail-closed terminate behavior for policy violations.
- invalid tool call repair if useful.

### Phase 6: Compaction Adapter

Do not remove current context editing immediately.

Initial approach:

- keep existing context editing and summarization middleware behavior.
- wrap it in a Rig-style compaction interface.
- use a `ConversationMemory` adapter only for loading and appending messages from existing stores.
- keep knowledge graph and memory fact writes in current services.

Later approach:

- introduce a durable compaction artifact store.
- distinguish ephemeral prompt summaries from persisted memory facts.
- bound summary artifacts explicitly.
- record compaction decisions as execution events for observability.

### Phase 7: Retire Framework Crates

Once runtime is stable:

- Replace `zero-agent` usages with Rig adapter APIs.
- Convert first-party tools away from `zero_core::Tool` or keep a compatibility wrapper.
- Shrink `zero-core` to shared data types only, or retire it.
- Rewrite `zero-app` prelude around the new adapter surface.
- Remove duplicate LLM abstractions if the Rig provider adapter becomes authoritative.

This should be the last phase, not the first.

## Multidimensional Analysis

### Maintainability

Gain: high.

Rig gives a clearer engine model than the current split between `zero-core`, `zero-agent`, `agent-runtime`, runtime hooks, gateway policy, and middleware. The most valuable simplification is collapsing duplicated framework concepts into one runner facade.

Risk: high during transition.

For a while, you will have both AgentZero runtime abstractions and Rig abstractions. Keep Rig behind an adapter to avoid making every crate understand both.

### Product Stability

Gain: medium.

Rig's run-state and streaming parity can make execution more predictable.

Risk: high if gateway events change.

The UI and gateway protocol are the product. The migration should be judged by whether existing sessions, delegation, streaming, and UI reducers still behave the same.

### Orchestration

Gain: high.

Rig's `AgentRun` gives a better substrate for continuations and resumability. Rig's hook `Flow` model gives a better policy vocabulary.

Risk: medium.

AgentZero already has richer product orchestration than Rig examples. Do not assume Rig replaces root/delegated/ward semantics. It should power them.

### Tooling

Gain: high.

Typed tools, dynamic tools, agent-as-tool, MCP, and hidden extensions are directly useful. This can rationalize first-party tools and reduce context leakage.

Risk: high.

Tool permissions, filesystem context, replay, large output handling, memory actions, connector auth, and actor policy are all subtle. This is the riskiest implementation surface after streaming.

### Memory And Knowledge

Gain: medium.

Rig's compaction interfaces can clean up the architecture. `ConversationMemory` can provide a better load/append boundary.

Risk: medium.

AgentZero's memory and knowledge systems are product-specific. Rig memory should adapt to them, not replace them.

### Provider Support

Gain: medium to high.

Rig has broad provider infrastructure and typed capability modeling.

Risk: medium to high.

Existing OpenAI-compatible config flexibility may be more important than compile-time provider modeling. Preserve current configs first.

### Performance

Gain: medium.

Rig's tool concurrency and shared streaming/non-streaming driver are mature. A cleaner state machine can reduce accidental work.

Risk: unknown.

Compile time, dependency size, provider adapter overhead, and stream adapter buffering need measurement. This should be benchmarked after a prototype.

### Security And Policy

Gain: medium.

Fail-closed hooks and hidden extensions are strong primitives.

Risk: high if policy moves into model-visible prompts or soft request overrides.

Executable tool filtering must remain a hard gateway/runtime boundary.

## Key Design Decisions

### Decision 1: Adapter First, Direct Replacement Later

Use `AgentExecutor` as the compatibility facade. The gateway should still ask for an executor and consume `StreamEvent`s.

Directly rewriting gateway-execution to call Rig would combine too many risks: engine behavior, event protocol, continuation, and tool policy.

### Decision 2: Preserve Configs Through Mapping

Current user config stays authoritative. The adapter maps it into Rig:

- agent instructions -> Rig preamble/static context.
- provider config -> Rig model/provider adapter.
- tool settings -> tool server construction and actor filtering.
- MCP config -> Rig MCP tools or current MCP adapter.
- memory settings -> context/memory adapter.

### Decision 3: Keep Gateway Events Stable

The adapter can emit legacy `StreamEvent`s even if Rig internally has richer events. Only add new events after existing UI reducers are stable.

### Decision 4: Treat Tool Exposure As A New Internal Contract

It is acceptable for tool exposure internals to change. The new contract should state:

- which actor can execute which tool.
- which schema is shown to the model.
- which hidden context is injected.
- which raw result is persisted.
- which transformed result is shown to the model.
- which result is shown to the UI.

### Decision 5: Compaction Should Be Layered

Rig-style compaction should become an interface inside AgentZero, not an immediate replacement for live context editing.

## Concrete Work Items

1. Create an ADR for "Rig-backed engine behind AgentExecutor".
2. Add contract tests for current event, delegation, continuation, and tool-policy behavior.
3. Add a small Rig adapter crate/module with no gateway type exposure.
4. Implement provider adapter over current OpenAI-compatible client stack.
5. Implement `zero_core::Tool` -> Rig tool bridge using `ToolCallExtensions`.
6. Implement Rig stream -> `agent_runtime::StreamEvent` adapter.
7. Implement current hook/middleware compatibility over Rig hooks.
8. Port one low-risk root-agent execution path behind a feature flag.
9. Port one delegated-agent path and verify `SessionContinuationReady`.
10. Port actor tool policy tests.
11. Add compaction adapter with current context editing behavior.
12. Only then plan retirement of `zero-agent`, `zero-core`, and duplicate prelude crates.

## Verification Gates

Run these before considering the migration viable:

- `cargo check --workspace`
- targeted runtime executor tests.
- gateway-execution tests around actor tool access.
- delegation and continuation tests.
- websocket event mapping tests.
- context editing and summarization tests.
- MCP tool listing/execution tests.
- memory and knowledge graph persistence tests.
- UI smoke test for streaming chat and research/delegation mode.

The highest-signal tests to preserve are the ones around `RuntimeActorKind`, `ExecutorBuilder`, `process_stream_event`, `convert_stream_event`, `SessionContinuationReady`, and websocket `gateway_event_to_server_message`.

## Recommendation

Replacing `zero-core` and `zero-agent` with Rig is reasonable if the real target is to simplify the engine under zbot, not to rewrite the product layer.

The right migration is:

1. Keep gateway/UI/memory/knowledge contracts stable.
2. Put Rig behind `AgentExecutor`.
3. Preserve current configs through adapter mapping.
4. Use Rig's state machine, hooks, tool server, hidden extensions, and memory interfaces to rationalize the runtime.
5. Retire framework crates after the active runtime path is proven.

What you gain is a cleaner engine core, better resumability, stronger tool abstractions, more coherent lifecycle policy, and a better compaction architecture. What you must protect is the product contract: gateway events, delegation semantics, actor permissions, memory/knowledge side effects, and UI behavior.
