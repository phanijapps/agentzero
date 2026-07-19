# Rig Comparison Notes

Date: 2026-06-27

Rig checkout: `rig checkout`, commit `6b1991bf`.

Codemem status:

- AgentZero was indexed with embeddings and a knowledge graph before this analysis.
- Rig was indexed with embeddings under Codemem project `rig`: 1,146 files, 24,162 chunks, 31,322 entity embeddings, 23,555 graph nodes, 26,059 graph edges.
- Both graphs have unresolved/external references, so graph evidence is useful for navigation, not as the sole basis for architectural conclusions.

## Executive Summary

Rig is most directly comparable to AgentZero's framework layer, especially `zero-core`, but Rig's mature pieces are not confined to a minimal core crate. Its strongest ideas cross `rig-core`, `rig-memory`, tool server code, provider clients, examples, and the agent run loop.

The most valuable lessons for AgentZero are:

1. Rig's strongly typed provider, tool, embedding, streaming, and vector-store boundaries are materially richer than `zero-core`'s current foundational traits.
2. Rig separates a serializable, sans-IO `AgentRun` state machine from the async `AgentRunner` driver. That pattern is more relevant to AgentZero gateway/runtime orchestration than to `zero-core` alone.
3. Rig's hook stack is a unified lifecycle-event model. AgentZero already has executor hooks and middleware, but they are split across config fields, middleware, tool events, and gateway-specific branches.
4. Rig's compaction is explicit memory shaping: window policies, demotion hooks, and compactor adapters. AgentZero has an always-on live context-control pipeline in runtime/gateway, with tool-result clearing and optional LLM summarization.
5. Rig should not be treated as a drop-in replacement. It is a strong reference for typed boundaries, state-machine shape, hook semantics, and compaction interfaces.

## Comparative Map

| Area | AgentZero Current Shape | Rig Shape | Where Rig Excels | Possible AgentZero Use |
| --- | --- | --- | --- | --- |
| Core agent abstraction | `zero_core::Agent` is a trait returning an event stream from `InvocationContext` (`framework/zero-core/src/agent.rs:15`). | Rig's built `Agent<M>` is a configured runtime object over a `CompletionModel`; `AgentBuilder` controls model, prompt, context, tools, hooks, memory, output schema, and turn defaults (`rig checkout/crates/rig-core/src/agent/builder.rs:96`). | Rig gives a concrete, ergonomic agent construction surface rather than only foundational traits. | Keep `zero-core` minimal, but consider a higher-level builder crate or runtime facade with stronger typed configuration. |
| Tool abstraction | `zero_core::Tool` is JSON-first: name, description, schemas, permissions, validation, `execute(ctx, Value)` (`framework/zero-core/src/tool.rs:28`). | Rig tools use associated `Args`, `Output`, and `Error`; dynamic dispatch bridges typed tools to provider schemas (`rig checkout/crates/rig-core/src/tool/mod.rs:116`). | Rig gets compile-time tool contracts and cleaner tool authoring. | Add typed-tool adapter APIs above the existing JSON `Tool` trait, rather than replacing current gateway tools. |
| Tool server and dynamic tools | AgentZero has `ToolRegistry` in runtime and capability-gated gateway registration. | Rig has a cloneable `ToolServerHandle`, dynamic tool sets, vector-retrieved tools, and agent-as-tool support (`rig checkout/crates/rig-core/src/tool/server.rs:143`, `rig checkout/crates/rig-core/src/tool/server.rs:226`, `rig checkout/crates/rig-core/src/agent/tool.rs:16`). | Rig treats tools as mutable runtime inventory and lets agents be tools while preserving runtime-only extensions. | Useful for ward agents, agent handoff, and dynamic capability discovery. |
| Provider model | AgentZero uses OpenAI-compatible runtime clients and gateway provider settings. | Rig models providers with capability markers and client extension traits; unsupported capabilities are unavailable at the type level. | Stronger compile-time expression of provider abilities. | Could inform `ModelRegistry`/provider capability modeling without pulling Rig's whole client stack in. |
| Completion and prompt APIs | AgentZero runtime loop directly drives `LlmClient::chat_stream` and tool loop (`runtime/agent-runtime/src/executor.rs:818`). | Rig layers `Prompt`, `Chat`, `TypedPrompt`, `CompletionModel`, request builders, and per-call overrides. | Cleaner provider-neutral request construction and structured-output controls. | Consider extracting request-building from AgentZero's executor loop. |
| Streaming | AgentZero streams token/reasoning/tool lifecycle events through `StreamEvent`. | Rig normalizes text, reasoning, tool-call deltas, final response, usage, message IDs, cancellation, and multi-turn streaming. | Rig has stronger streaming parity between blocking and streaming drivers. | Useful reference for replayable WebSocket/event history and consistent tool-call deltas. |
| Embeddings/vector stores | AgentZero stores and services implement persistence and memory search across multiple crates. | Rig exposes `EmbeddingModel`, `Embed`, `EmbeddingsBuilder`, `VectorStoreIndex`, and vector stores that can become tools. | Rig's embedding/vector abstractions are a clean core API, not just storage plumbing. | Consider aligning `zero-core` or `zbot-stores-traits` with typed embedding/vector interfaces. |
| Run-loop state | AgentZero's `AgentExecutor` owns the loop, messages, hooks, streaming, tool execution, context editing, recall, steering, and limits in one async driver (`runtime/agent-runtime/src/executor.rs:520`). | Rig's `AgentRun` is a serializable, sans-IO state machine; `AgentRunner` owns IO, hooks, memory, model calls, tools, and telemetry (`rig checkout/crates/rig-core/src/agent/run/mod.rs:1`, `rig checkout/crates/rig-core/src/agent/runner.rs:220`). | This is Rig's most important architectural advantage for durable/resumable orchestration. | Strong candidate for gateway execution and continuation redesign. |
| Hooks/policy | AgentZero has `before_tool_call`, `after_tool_call`, `transform_context`, middleware, and gateway policy branches (`runtime/agent-runtime/src/executor.rs:168`). | Rig uses one hook method over `StepEvent` variants with `Flow` decisions such as continue, skip, rewrite args/result, override request, retry/repair invalid tool calls, or terminate (`rig checkout/crates/rig-core/src/agent/hook.rs:1`). | Unified lifecycle hooks reduce special-case branches and make policy composition explicit. | Adopt the event/flow model conceptually for gateway policy and UI approvals. |
| Context compaction | AgentZero has live middleware: context editing clears old tool results, plan-block reinsertion pins state, optional summarization follows (`gateway/gateway-execution/src/invoke/executor.rs:260`). | Rig has explicit memory policies and compaction adapters around conversation memory (`rig checkout/crates/rig-memory/src/lib.rs:11`). | Rig's compaction interfaces are composable and testable; AgentZero's live context-control path is more automatic. | Borrow interfaces, not behavior wholesale. |

## Zero-Core Fit

`zero-core` currently defines foundational traits and types: `Agent`, `Tool`, `Toolset`, contexts, events, capabilities, permissions, and registries (`framework/zero-core/src/lib.rs:45`). The `Agent` trait is deliberately small: name, description, sub-agents, and `run(ctx) -> EventStream` (`framework/zero-core/src/agent.rs:20`). The `Tool` trait is similarly small and JSON-oriented (`framework/zero-core/src/tool.rs:33`).

Rig's `rig-core` has a broader definition of "core". It includes agent construction, completion models, typed tools, embeddings, vector stores, memory traits, provider clients, streaming, and a run state machine. That means Rig is aligned with the intent of `zero-core`, but not with its current scope.

Where Rig is ahead for a `zero-core` comparison:

- Typed tools with associated input/output/error types and dynamic dispatch bridges.
- Provider capability modeling as part of client construction.
- Provider-neutral message and streaming primitives with richer multimodal/reasoning/tool-call support.
- Embedding and vector-store traits that are first-class framework APIs.
- Agent builder typestate that prevents invalid tool configuration combinations.
- Agent-as-tool support with runtime-only context propagation through `ToolCallExtensions` (`rig checkout/crates/rig-core/src/agent/tool.rs:47`).

Where AgentZero should be cautious:

- `zero-core` is a bottom-layer crate used across the workspace. Pulling Rig-style provider clients or memory policy directly into it would expand its dependency and ownership surface.
- Rig's typed APIs are Rust-ergonomic, but AgentZero gateway and UI surfaces already need JSON, persistence, OpenAI-compatible APIs, and dynamic tool loading.
- A better target may be new typed adapters above `zero-core`, leaving the existing `Tool` and `Agent` traits as stable interoperability boundaries.

## Gateway And Runtime Orchestration Fit

AgentZero gateway/runtime orchestration is more complex than `zero-core`: actor kinds, capability-gated tool registration, delegation, wait/kill/steer/handoff tools, continuation watchers, context control, recall, and execution-state persistence.

Current AgentZero evidence:

- `RuntimeActorKind` separates root, delegated executor, delegated reviewer, and ward agent (`gateway/gateway-execution/src/invoke/executor.rs:67`).
- Tool access is capability-gated by actor kind, with tests asserting ordinary subagents do not receive orchestration tools and ward/root agents do (`gateway/gateway-execution/src/invoke/executor.rs:1193`).
- Runtime has hooks for before-tool, after-tool, context transform, tool execution mode, turn budgets, max turns, and single-action mode (`runtime/agent-runtime/src/executor.rs:168`).
- The executor handles streaming, context warning, recall injection, steering injection, tool calls, parallel/sequential execution, action events, result shaping, and message mutation in one loop (`runtime/agent-runtime/src/executor.rs:579`).

Rig patterns that could improve this layer:

1. **Serializable run state**
   Rig's `AgentRun` carries pending tool calls inside serializable state so a resumed process can re-obtain them (`rig checkout/crates/rig-core/src/agent/run/mod.rs:241`). AgentZero continuation, wait-agent, daemon restart, human approval, and WebSocket reconnect behavior would benefit from a similar state-machine boundary.

2. **Unified lifecycle hook stack**
   Rig has one `AgentHook::on_event` over `CompletionCall`, `CompletionResponse`, `ToolCall`, `ToolResult`, streaming deltas, and invalid tool calls (`rig checkout/crates/rig-core/src/agent/hook.rs:31`). AgentZero's policies currently live in several extension points and gateway branches. A common event/flow model could unify logging, policy, UI approval, request overrides, context transforms, and tool-result redaction.

3. **Request override flow**
   Rig hooks can override per-turn request fields such as active tools, tool choice, sampling, prompt, and provider params (`rig checkout/crates/rig-core/src/agent/hook.rs:291`). Tool-choice behavior is provider-dependent, so this should not be the only safety boundary. AgentZero could use request overrides for phase-based orchestration while enforcing safety through gateway policy and executable tool filtering.

4. **Deterministic concurrent tools**
   Rig documents that concurrent tools may complete out of order, but final message history is call-ordered (`rig checkout/crates/rig-core/src/agent/runner.rs:329`). AgentZero already executes tools with `join_all` and processes results in original tool-call order (`runtime/agent-runtime/src/executor.rs:1017`). This is an area where AgentZero is aligned; Rig can serve as validation and test inspiration.

5. **Agent-as-tool with hidden extensions**
   Rig lets an agent implement `Tool` and propagates runtime-only extensions into the sub-agent (`rig checkout/crates/rig-core/src/agent/tool.rs:47`). AgentZero's ward and subagent architecture could use a similar hidden-context channel for session IDs, actor kind, auth scopes, ward IDs, trace IDs, and policy context.

## Rig Orchestration Patterns Worth Studying

Rig's examples are useful because they are small, concrete compositions rather than a single heavyweight orchestrator:

| Pattern | Rig Evidence | AgentZero Relevance |
| --- | --- | --- |
| Task decomposition and judge synthesis | `agent_orchestrator` classifies/extracts tasks, runs one content agent per task, then sends collected outputs to a judge (`rig checkout/examples/agent_orchestrator/src/main.rs:31`). | Similar shape to root agent delegation plus continuation, but could be modeled as an explicit run graph/state machine. |
| Prompt chaining | `agent_prompt_chaining` feeds one agent's output into another (`rig checkout/examples/agent_prompt_chaining/src/main.rs:32`). | Useful for deterministic gateway workflows where full open-ended delegation is too much. |
| Parallel evaluation | `agent_parallelization` runs scoring agents concurrently and keeps individual `Result`s (`rig checkout/examples/agent_parallelization/src/main.rs:45`). | AgentZero already supports parallel tool execution and delegation; this suggests making partial-failure semantics first-class in orchestration docs. |
| Evaluator/optimizer loop | `agent_evaluator_optimizer` loops generator output through structured evaluation until pass/fail criteria are met (`rig checkout/examples/agent_evaluator_optimizer/src/main.rs:67`). | Strong fit for future quality gates, self-review loops, and automated retry policies. |
| Agent-as-tool | `agent_with_agent_tool` registers an agent as a tool of another agent; core `Tool for Agent<M>` implements this pattern (`rig checkout/crates/rig-core/src/agent/tool.rs:16`). | Maps to ward agents and specialized subagents, but AgentZero must preserve actor-kind authorization and session boundaries. |
| Human/policy approval | Rig examples model approvals as tool-call hooks that can continue, skip, rewrite args, or terminate (`rig checkout/examples/agent_with_human_in_the_loop/src/main.rs:142`, `rig checkout/examples/agent_with_approval_policy/src/main.rs:113`); core enforces those decisions before execution (`rig checkout/crates/rig-core/src/agent/runner.rs:450`). | Good model for gateway UI approvals and policy-driven tool denial without scattering checks through tool implementations. |

The common thread is that Rig makes orchestration mechanics explicit at the agent-run layer: state machine, hook events, and typed flows. AgentZero currently has richer product orchestration, but the control flow is distributed across runtime executor branches, gateway continuation watchers, event bus handling, and tool result conventions. Rig's architectural value is therefore not "more orchestration features"; it is a cleaner place to put orchestration decisions.

## Rig Compaction Model

Rig compaction lives in conversation memory, not as a global always-on request preprocessor.

Core pieces:

- `ConversationMemory` loads ordered messages before a prompt and appends successful turns afterward (`rig checkout/crates/rig-core/src/memory.rs:85`).
- `rig-memory` provides `MemoryPolicy`, `SlidingWindowMemory`, `TokenWindowMemory`, demotion hooks, `CompactingMemory`, and `TemplateCompactor` (`rig checkout/crates/rig-memory/src/lib.rs:11`).
- `Compactor` converts evicted messages plus optional carry-over into a single `Message`-shaped artifact, producing a prompt shape of `[summary, ...recent_window]` (`rig checkout/crates/rig-core/src/memory.rs:264`).
- `TemplateCompactor` is non-LLM and can bound summary size with `with_max_bytes`; custom LLM compactors are extension points, not a built-in framework default.
- Rig's compaction caveats matter: per-conversation watermarks and carry-over summaries are in-process unless the caller builds durable deduplication, and the spliced summary can sit outside a memory policy's budget unless the compactor itself bounds the artifact.

Important contrast with AgentZero:

- AgentZero's runtime context editing is live and threshold-triggered. It estimates tokens, clears old tool result content, preserves recent tool results, can cascade unload skill resources, and emits an event (`runtime/agent-runtime/src/middleware/context_editing.rs:402`).
- AgentZero's optional summarization middleware calls an LLM, excludes system/tool/prior-summary messages, and inserts a system summary before non-system kept messages (`runtime/agent-runtime/src/middleware/summarization.rs:120`).
- Gateway wires context editing before plan-block reinsertion, and optional summarization after the plan block (`gateway/gateway-execution/src/invoke/executor.rs:260`).

Inference: Rig's compaction is cleaner as a composable memory abstraction, with explicit durability and budget caveats; AgentZero's path is stronger as a live agent-session survival mechanism. The best direction is likely to introduce Rig-like memory policy/compactor interfaces behind AgentZero's existing live middleware, not to replace live context editing with Rig's explicit memory policies.

## Aggregate Recommendations

### Near-Term Documentation And Design

- Document a desired split between `AgentRunState` and `AgentRunner` in AgentZero gateway/runtime. The state object should own turn count, pending tool calls, current message history, usage, and continuation state; the runner should own IO.
- Document a unified lifecycle event model for gateway policy. Start with events equivalent to model-call, model-response, tool-call, tool-result, stream-delta, invalid-tool-call, delegation-start, delegation-complete, and continuation-ready.
- Add a short architecture note that `zero-core` remains the stable minimal trait layer, while typed providers/tools/builders live above it.

### Candidate Implementation Spikes

1. **Typed tool adapter**
   Build a `TypedTool<Args, Output, Error>` adapter that emits the existing `zero_core::Tool` JSON surface. This captures Rig's tool ergonomics without breaking dynamic gateway tools.

2. **Run-state prototype**
   Extract a serializable runtime state struct from `AgentExecutor` for one narrow path: pending tool calls and next-step transitions. Prove it can pause before tool execution and resume.

3. **Lifecycle hook enum**
   Add an internal enum and flow result that can wrap the existing `before_tool_call`, `after_tool_call`, and `transform_context` hooks. Keep current hooks as compatibility shims.

4. **Memory compactor interface**
   Add a `Compactor`-like trait near memory/context-control code that can turn evicted context into a bounded artifact. Let existing context editing call it only after tool-result clearing cannot reclaim enough.

5. **Agent-as-tool experiment**
   Model a ward agent or subagent as a tool-like capability with hidden runtime context propagation. Keep actor-kind authorization in gateway, not in prompt text.

## Explicit Non-Recommendations

- Do not vendor or directly replace `zero-core` with Rig. Rig's core is broader and would move provider/runtime/memory concerns into AgentZero's bottom layer.
- Do not remove AgentZero's live context editing. Rig does not appear to provide an always-on context-window manager equivalent to AgentZero's runtime middleware.
- Do not copy Rig's provider stack wholesale before deciding whether AgentZero wants compile-time provider capability modeling or runtime-configurable OpenAI-compatible flexibility.
- Do not treat Rig's `TemplateCompactor` as equivalent to AgentZero summarization. It is a deterministic rollup reference implementation, not an LLM summarizer.

## Open Questions

- Should AgentZero's typed tool adapter live in `zero-core`, `zero-tool`, or `agent-runtime`?
- Should gateway continuation state become a serializable run-state boundary, or should it stay in execution-state plus event streams?
- Should compaction artifacts be persisted into `zbot-stores`/knowledge memory, or remain ephemeral context-control artifacts?
- Should provider capabilities be enforced statically in Rust or dynamically through `ModelRegistry` and provider config?
