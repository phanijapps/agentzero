# daemon (zbotd)

Standalone server binary that runs the full AgentZero platform. Composes the `gateway` crate into a runnable process with logging, config loading, and jemalloc as the global allocator.

## Binary Name

```
zbotd
```

## CLI Options

| Flag | Default | Purpose |
|------|---------|---------|
| `--port` / `--http-port` | 18791 | HTTP port |
| `--ws-port` | 18790 | WebSocket port |
| `--data-dir` | `~/Documents/agentzero` | Data directory (vault) |
| `--config` | — | Optional YAML config file |
| `--static-dir` | — | Serve React dashboard from this path |
| `--no-dashboard` | false | Disable dashboard serving |
| `--log-dir` | — | Enable file logging, write to this directory |
| `--log-max-files` | 7 | Rolling log file count |

## Logging

Configurable via `settings.json` in the data directory or CLI flags (CLI overrides settings.json):

```json
{
  "logs": {
    "enabled": true,
    "level": "info",
    "rotation": "daily",
    "maxFiles": 7,
    "suppressStdout": false
  }
}
```

## Source

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI arg parsing, logging setup, `GatewayServer::start()` |
| `build.rs` | Build script (embed version info) |

## Intra-Repo Dependencies

- `gateway` — `GatewayConfig`, `GatewayServer`
- `gateway-services` — `AppSettings`, `LogSettings`
- `gateway` — HTTP/WebSocket shell and runtime wiring

## Notes

- Uses `tikv-jemallocator` as global allocator to prevent RSS bloat from ONNX model cycles.
- The React dashboard is built separately (`apps/ui`) and served via `--static-dir ./dist`.
- Development: `npm run daemon:watch` compiles and runs with file-change watching.
