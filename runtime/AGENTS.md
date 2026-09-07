# Runtime

Shared primitives, Rig-backed execution, and built-in tools.

## Crates

| Crate | Purpose |
|-------|---------|
| `agent-primitives` | Shared tool/context/event/content/error primitives used by runtime, tools, gateway, and stores. No agent engine lives here. |
| `agent-runtime` | LLM client, middleware, tool registry, legacy executor facade, and Rig adapter/engine selection. |
| `agent-tools` | Built-in tool implementations exposed through `agent_primitives::Tool`. |

## agent-runtime

The runtime keeps zbot's gateway-facing execution contract stable while Rig owns the selectable agent loop behind the adapter.

- **Engine facade**: `AgentEngine` is the boundary consumed by `gateway-execution`.
- **Rig path**: `rig_adapter::RigAgentEngine` maps zbot config, tools, hooks, history, and stream items into existing `StreamEvent`s.
- **Engine**: Rig is the sole engine. `gateway-execution` constructs `RigAgentEngine` unconditionally from prepared session inputs; there is no engine-selection flag.
- **Provider bridge**: `rig_adapter::LlmCompletionModel` adapts Rig completion calls onto zbot's existing OpenAI-compatible `LlmClient`, retry, and rate-limit stack.
- **Tool bridge**: `rig_adapter::RigToolAdapter` adapts `agent_primitives::Tool` into Rig tool dispatch and carries hidden runtime context through Rig extensions.
- **Fallback limits**: sessions with MCP servers still use the legacy executor path until MCP lifecycle cleanup is bridged.
- **Middleware**: summarization, context editing, token counting, recall, and gateway-owned orchestration remain zbot-owned.

Key files:

| File | Purpose |
|------|---------|
| `src/executor.rs` | Existing executor and `AgentEngine` facade implementation. |
| `src/rig_adapter/engine.rs` | Rig-backed engine and stream/event mapping. |
| `src/rig_adapter/model.rs` | Rig `CompletionModel` bridge over zbot `LlmClient`. |
| `src/rig_adapter/tool.rs` | Rig tool bridge over `agent_primitives::Tool`. |
| `src/llm/openai.rs` | OpenAI-compatible streaming client. |
| `src/middleware/` | zbot-owned context and compaction middleware. |

## agent-tools

Built-in tools are organized into core and optional sets, then registered by the gateway runner according to actor policy and settings.

### Core Tools

| Tool | Description |
|------|-------------|
| `shell` | Run shell commands with the configured ward/session context. |
| `write_file` | Create or overwrite a file. |
| `edit_file` | Targeted edits in existing files. |
| `memory` | Durable memory and graph operations through zbot stores. |
| `ward` | Manage code wards. |
| `update_plan` | Lightweight task checklist. |
| runtime title service | Sets human-readable session labels without a model-visible tool call. |
| `execution_graph` | DAG workflow helper. |
| `load_skill` | Load bounded skill packets and section handles for known skill names. |

### Action Tools

| Tool | Description |
|------|-------------|
| `respond` | Send a response through gateway-owned hooks. |
| `delegate_to_agent` | Spawn delegated execution through gateway orchestration. |
| `list_agents` | List available agents. |

## Does Not Own

- HTTP/WebSocket routing and event fan-out: `gateway/`.
- Session/execution persistence: `services/execution-state` and `stores/zbot-stores-sqlite`.
- Durable semantic memory and knowledge graph ownership: `stores/*`, `services/knowledge-graph`, and `gateway-memory`.
