# agent-runtime

Runtime execution crate for zbot. It owns the gateway-facing execution facade, the Rig adapter, LLM clients, middleware, tool registry, and MCP manager.

## Build & Test

```bash
cargo test -p agent-runtime
cargo build -p agent-runtime
```

## Current Engine Shape

`gateway-execution` consumes `AgentEngine`, not Rig directly. This crate provides two implementations behind that facade:

| Engine | Purpose |
|--------|---------|
| `AgentExecutor` | Existing executor implementation and fallback path. |
| `RigAgentEngine` | Rig-backed implementation that adapts zbot config/tools/history/hooks/streams into the existing runtime event contract. |

`ZBOT_ENGINE=rig` selects the Rig path when gateway safety gates allow it. Sessions with configured MCP servers currently fall back to `AgentExecutor` because MCP subprocess lifecycle cleanup has not been moved into the Rig path.

## Key Components

| File | Purpose |
|------|---------|
| `executor.rs` | `AgentExecutor`, `AgentEngine` facade, existing stream execution path. |
| `rig_adapter/engine.rs` | Rig-backed engine, hook mapping, stream mapping, stop handling. |
| `rig_adapter/model.rs` | Rig `CompletionModel` implementation over zbot's `LlmClient`. |
| `rig_adapter/tool.rs` | Rig `ToolDyn` bridge over `agent_primitives::Tool`. |
| `rig_adapter/config.rs` | Neutral Rig-facing config resolved from existing zbot settings. |
| `llm/client.rs` | `LlmClient` trait: `chat()` and `chat_stream()`. |
| `llm/openai.rs` | OpenAI-compatible streaming client and request encoding. |
| `llm/retry.rs` | Retrying LLM wrapper. |
| `types/events.rs` | `StreamEvent` contract consumed by gateway. |
| `tools/registry.rs` | Runtime tool registry. |
| `mcp/` | MCP manager for the fallback executor path. |
| `middleware/` | Summarization, context editing, token counting, and related runtime context control. |

## Event Contract

Both engine paths must emit existing `StreamEvent` variants so gateway conversion and UI reducers remain unchanged: token/reasoning deltas, tool lifecycle, respond/delegate actions, ward changes, token updates, completion, errors, and UI interactions.

## Code Style

- Keep direct `rig` imports inside `rig_adapter/`.
- Keep gateway-visible contracts in zbot types.
- Use `Arc<T>` for shared state that crosses async boundaries.
- Return typed errors; do not use stringly placeholder errors for new runtime code.
