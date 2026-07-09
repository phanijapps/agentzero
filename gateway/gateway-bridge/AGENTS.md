# gateway-bridge

WebSocket worker system for bidirectional agent-worker communication. Workers (any language) connect via WebSocket, self-describe their capabilities and resources, and receive outbound pushes in real time.

Also manages STDIO plugins (Node.js extensions) via `PluginManager`.

## Key Exports

```rust
pub use error::BridgeError;
pub use handler::handle_worker_connection;      // per-worker WS session loop
pub use outbox::OutboxRepository;               // SQLite-backed reliable delivery
pub use pending_requests::PendingRequests;
pub use plugin_config::{PluginConfig, PluginError, PluginState, PluginSummary, PluginUserConfig};
pub use plugin_manager::PluginManager;
pub use protocol::{BridgeServerMessage, WorkerCapability, WorkerMessage, WorkerResource};
pub use provider::BridgeResourceProvider;       // ConnectorResourceProvider impl
pub use push::{enqueue_and_push, spawn_retry_loop};
pub use registry::{BridgeRegistry, WorkerSummary};
pub use stdio_plugin::StdioPlugin;
```

## Modules

| Module | Purpose |
|--------|---------|
| `protocol` | `WorkerMessage` (worker→server) and `BridgeServerMessage` (server→worker) types |
| `registry` | In-memory `BridgeRegistry` tracking connected workers |
| `outbox` | SQLite-backed outbox with ACK tracking for reliable push delivery |
| `push` | `enqueue_and_push()` + `spawn_retry_loop()` — drain and retry unacknowledged pushes |
| `handler` | Per-worker WebSocket session loop: auth, capability registration, message dispatch |
| `provider` | `BridgeResourceProvider` — `ConnectorResourceProvider` impl for bridge workers |
| `pending_requests` | Track in-flight requests awaiting worker responses |
| `plugin_config` | `PluginConfig` — user-facing plugin configuration |
| `plugin_manager` | `PluginManager` — spawn and supervise STDIO plugin processes |
| `stdio_plugin` | `StdioPlugin` — individual STDIO plugin process wrapper |

## Architecture

```
Workers (any language)
       │  WebSocket
       ▼
  BridgeRegistry  ←→  OutboxRepository (SQLite)
       │                     │
  handle_worker_connection   spawn_retry_loop
       │
  BridgeResourceProvider → ConnectorRegistry
```

## Intra-Repo Dependencies

- `gateway-services` — `VaultPaths`, plugin config loading
- `zbot-stores-sqlite` — outbox SQLite operations

## Notes

- Workers self-register capabilities and resources on connect.
- Outbox guarantees at-least-once delivery with ACK; replay loop retries on reconnect.
- STDIO plugins are Node.js processes managed by `PluginManager` via stdin/stdout JSON protocol.
