# agent-runtime

Runtime execution crate for zbot. It owns the gateway-facing execution facade, the Rig adapter, LLM clients, middleware, tool registry, and MCP manager.

## Build & Test

```bash
cargo test -p agent-runtime
cargo build -p agent-runtime
```

## Current Engine Shape

`gateway-execution` consumes `AgentEngine`, not Rig directly. The sole
implementation behind that facade:

| Engine | Purpose |
|--------|---------|
| `RigAgentEngine` | Adapts zBot config/tools/history/hooks/streams into the runtime event contract; owns configured MCP session lifetimes. |

`gateway-execution` constructs it unconditionally from prepared session inputs
(`build_execution_engine`); there is no engine-selection flag and no fallback.

## Key Components

| File | Purpose |
|------|---------|
| `engine/` | Neutral facade: `AgentEngine` trait, `ExecutorConfig`, `ExecutorError`, `EngineHook` + `HookSet`, `PreparedExecution`, private snapshots. |
| `engine/hooks.rs` | The hook framework: `EngineHook` trait (defaulted methods), `HookSet` (ordered multi-slot fan-out), `ToolDecision`, `RecallPacket`. |
| `rig_adapter/engine.rs` | The engine: hook mapping, stream mapping, stop handling, turn signal. |
| `rig_adapter/turn_signal.rs` | `TurnSignal` enum: Continue, Stop, DelegationYield, Responded, TurnLimit. |
| `rig_adapter/turn_events.rs` | Pure functions mapping `MultiTurnStreamItem` → `StreamEvent`. |
| `rig_adapter/model.rs` | Rig `CompletionModel` implementation over zbot's `LlmClient`. |
| `rig_adapter/tool.rs` | Rig `ToolDyn` bridge over `agent_primitives::Tool`. |
| `rig_adapter/config.rs` | Neutral Rig-facing config resolved from existing zbot settings. |
| `rig_adapter/factory.rs` | `build_engine()` — constructs Rig from `PreparedExecution`. |
| `llm/client.rs` | `LlmClient` trait: `chat()` and `chat_stream()`. |
| `llm/openai.rs` | OpenAI-compatible streaming client, request encoding, strict JSON schema normalization, reasoning_content fallback. |
| `llm/retry.rs` | Retrying LLM wrapper. |
| `types/events.rs` | `StreamEvent` contract consumed by gateway. |
| `tools/registry.rs` | Runtime tool registry. |
| `tools/context.rs` | `ToolContext` shared across tool executions. |
| `mcp/` | MCP transports and session ownership consumed by the Rig engine. |
| `middleware/` | Summarization, context editing, token counting, and related runtime context control. |

## Event Contract

The engine must emit the existing `StreamEvent` variants so gateway conversion and UI reducers remain unchanged: token/reasoning deltas, tool lifecycle, respond/delegate actions, ward changes, token updates, completion, errors, and UI interactions.

## Code Style

- Keep direct `rig` imports inside `rig_adapter/`.
- Keep gateway-visible contracts in zbot types.
- Use `Arc<T>` for shared state that crosses async boundaries.
- Return typed errors; do not use stringly placeholder errors for new runtime code.
