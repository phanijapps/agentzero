# Gateway

Network layer providing HTTP and WebSocket APIs for agent interaction. Decomposed into 11 focused sub-crates with a thin shell for HTTP routes, WebSocket handler, and AppState wiring.

## Crate Structure

```
gateway/
├── gateway-events/      # EventBus, GatewayEvent (25+ variants), HookContext, HookType
├── gateway-templates/   # System prompt assembly (SOUL + INSTRUCTIONS + OS + shards)
├── gateway-connectors/  # ConnectorRegistry, dispatch to external services
├── gateway-services/    # AgentService, ProviderService, McpService, SkillService, SettingsService,
│                        #   EmbeddingService, ModelRegistry, PluginService, VaultPaths
├── gateway-execution/   # ExecutionRunner, delegation, lifecycle, streaming, BatchWriter
├── gateway-hooks/       # Hook trait, HookRegistry, CliHook, CronHook, NoOpHook
├── gateway-cron/        # CronJobConfig, CronService, CRUD types
├── gateway-bus/         # GatewayBus trait, SessionRequest, SessionHandle
├── gateway-ws-protocol/ # ClientMessage, ServerMessage, SubscriptionScope
├── gateway-bridge/      # WebSocket worker protocol, OutboxRepository, PluginManager, StdioPlugin
├── src/                 # Thin shell: HTTP routes, WS handler, AppState
│   ├── http/            #   REST API routes (agents, providers, skills, plugins, artifacts, etc.)
│   ├── websocket/       #   WebSocket handler + subscription manager
│   ├── bus/             #   HttpGatewayBus (composes execution runner with bus trait)
│   ├── hooks/           #   WebHook (depends on WS module)
│   └── state/           #   AppState + persistence_factory (wires all services together)
└── templates/           # Embedded system prompt templates (SOUL, INSTRUCTIONS, OS, shards)
```

## Sub-Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `gateway-events` | Event bus (broadcast), `GatewayEvent` variants, `HookContext` |
| `gateway-templates` | Prompt assembly from config/ files + embedded shards; auto-creates defaults |
| `gateway-connectors` | HTTP/CLI connectors for dispatching agent responses outbound |
| `gateway-services` | Config services with `RwLock` caching (agents, providers, MCPs, skills, settings, embeddings) |
| `gateway-execution` | Execution engine: runner, delegation, lifecycle, stream, `BatchWriter`, distillation |
| `gateway-hooks` | `Hook` trait + registry (CLI, Cron, NoOp implementations) |
| `gateway-cron` | Cron job config types and CRUD service |
| `gateway-bus` | `GatewayBus` trait, `SessionRequest`/`Handle` types |
| `gateway-ws-protocol` | WebSocket message types (Client/Server), `SubscriptionScope` |
| `gateway-bridge` | Worker WebSocket protocol, SQLite-backed outbox, `PluginManager`, `StdioPlugin` |
| `gateway` (shell) | HTTP routes, WS handler, AppState, static file serving, OpenAPI spec |

## Dependency Direction

```
gateway-events (foundation — no gateway deps)
    ├── gateway-templates
    ├── gateway-connectors
    ├── gateway-services
    ├── gateway-hooks
    ├── gateway-cron
    ├── gateway-bus
    ├── gateway-ws-protocol
    ├── gateway-bridge (depends on gateway-services + zbot-stores-sqlite)
    └── gateway-execution (depends on most above)
            │
            └── gateway (thin shell — composes everything)
```

## Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 18791 | HTTP + WS | REST API, static files, and WebSocket at `/ws` (primary port) |
| 18790 | WS | Legacy standalone WebSocket — only active with `--legacy-ws-port-enabled`; deprecated, slated for removal |

## HTTP Endpoints (partial)

- `GET|POST|PUT|DELETE /api/agents[/:id]` — Agent CRUD
- `GET|POST /api/providers[/:id]` — Provider config
- `GET|POST|PUT|DELETE /api/connectors[/:id]` — Connector CRUD
- `GET|POST|PUT|DELETE /api/cron[/:id]` — Cron job CRUD
- `GET|POST /api/skills` — Skill listing/creation
- `GET|POST /api/mcps` — MCP configs
- `POST /api/gateway/submit` — Submit agent request
- `GET /api/gateway/status/:session_id` — Session status
- `POST /api/gateway/cancel/:session_id` — Cancel session
- `GET /api/executions/v2/sessions/full` — All sessions with executions
- `GET /api/logs/sessions` — Execution log sessions
- `GET /api/plugins` — Plugin listing
- `GET|POST /api/artifacts` — Ward artifact management

## WebSocket Protocol

```typescript
// Client → Server (ClientMessage)
{ type: "invoke", agent_id, conversation_id, message, session_id?, mode? }
{ type: "stop", conversation_id }
{ type: "continue", conversation_id, additional_iterations? }
{ type: "subscribe", conversation_id, scope: "all"|"session"|{execution: id} }
{ type: "pause"|"resume"|"cancel", session_id }
{ type: "end_session", session_id }
{ type: "ping" }

// Server → Client (GatewayEvent)
agent_started, agent_completed, agent_stopped, token, thinking, tool_call, tool_result,
turn_complete, delegation_started, delegation_completed, session_continuation_ready,
ward_changed, plan_update, iterations_extended, session_title_changed,
intent_analysis_started, intent_analysis_complete, token_usage, heartbeat, error
```

## Key Patterns

- **BatchWriter**: Decouples DB writes from streaming callback (100ms flush, token coalescing)
- **RwLock caching**: Provider, MCP, Settings services cache config files in memory
- **Synchronous event publish**: `publish_sync` for token ordering preservation
- **Session ward_id**: Persisted to DB on `WardChanged`, restored on continuation/delegation
- **gateway-bridge outbox**: SQLite-backed reliable delivery for worker pushes with ACK/retry

## Dependencies

- `runtime/*` — Agent executor, LLM client, tools
- `services/*` — Execution state, API logs
- `stores/zbot-stores-sqlite` — SQLite connection pool, schema, and repositories
- `runtime/agent-primitives` — `FileSystemContext` trait and shared tool primitives
- `discovery` — LAN mDNS advertisement
