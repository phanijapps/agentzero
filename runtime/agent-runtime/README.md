# Agent Runtime Library

`agent-runtime` is zbot's execution runtime. It keeps the gateway-facing execution contract stable and implements it with the Rig-backed engine behind an adapter.

## Overview

The crate owns:

- `AgentEngine`, the facade used by `gateway-execution`.
- `RigAgentEngine`, the sole engine implementation.
- `LlmClient` and OpenAI-compatible provider transport.
- Runtime middleware for context control and token accounting.
- Tool registry/context types used by built-in tools and the Rig tool bridge.
- MCP transports and session ownership consumed by the Rig engine.

Rig is intentionally contained under `src/rig_adapter/`. Gateway, stores, tools, and UI code should not depend on Rig types directly.

## Architecture

```
gateway-execution
        │
        ▼
  AgentEngine facade
        │
        └── RigAgentEngine
              ├── LlmCompletionModel -> zbot LlmClient
              ├── RigToolAdapter -> agent_primitives::Tool
              └── Rig hooks/stream items -> zbot StreamEvent
```

## Usage

Production construction happens in `gateway-execution`, which selects the engine and wires provider config, tools, hooks, memory context, and cancellation. Library users should prefer the facade boundary rather than constructing Rig types directly.

## License

MIT
