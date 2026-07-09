# cli (zbot)

Terminal UI client for interacting with AgentZero agents. Connects to a running daemon over HTTP + WebSocket.

## Binary Name

```
zbot
```

## Subcommands

| Command | Purpose |
|---------|---------|
| `chat [agent]` | Interactive TUI chat (default agent: `assistant`) |
| `invoke <agent> <message>` | Send a single message, print response, exit |

## Global Flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--port` | 18791 | Gateway HTTP port |
| `--host` | 127.0.0.1 | Gateway host |

## Source Structure

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI arg parsing, subcommand dispatch |
| `src/app.rs` | TUI application state machine |
| `src/client.rs` | HTTP + WebSocket client (`reqwest` + `tokio-tungstenite`) |
| `src/events.rs` | Event processing from the WebSocket stream |
| `src/ui.rs` | `ratatui` rendering |

## Tech Stack

- `ratatui` + `crossterm` — terminal UI rendering
- `tokio-tungstenite` — WebSocket streaming
- `reqwest` — HTTP requests
- `clap` — CLI argument parsing

## Intra-Repo Dependencies

- `agent-primitives` — shared types only (no runtime dependency)

## Notes

- Connects to a running `zbotd` instance; does not embed the runtime.
- WebSocket stream delivers `GatewayEvent` JSON for real-time token streaming.
- Build: `cargo build -p cli` → produces `zbot` binary.
