# Apps

Runnable applications built on the gateway.

## Structure

```
apps/
├── daemon/     # HTTP/WebSocket server (zbotd binary)
├── cli/        # Terminal UI client (zbot binary)
└── ui/         # Web dashboard (React/TypeScript)
```

## Daemon (zbotd)

Standalone HTTP/WebSocket server running the full agent platform.

```bash
# Run with dashboard
cargo run -p daemon -- --static-dir ./dist

# Run on custom port
cargo run -p daemon -- --port 8080

# Run with data directory
cargo run -p daemon -- --data-dir ~/.agentzero
```

**CLI Options:**
- `--static-dir` — Path to React dashboard dist/ directory
- `--port` / `--http-port` — HTTP port (default: 18791)
- `--ws-port` — WebSocket port (default: 18790)
- `--data-dir` — Data directory / vault (default: `~/Documents/zbot`)
- `--log-dir` — Enable file logging
- `--no-dashboard` — Disable static file serving

See `apps/daemon/AGENTS.md` for full details.

## CLI (zbot)

Terminal UI client for interacting with agents.

```bash
cargo run -p cli -- chat assistant
cargo run -p cli -- invoke assistant "What time is it?"
```

See `apps/cli/AGENTS.md` for full details.

## UI

Web dashboard. React 19 + TypeScript + Vite. See [ui/AGENTS.md](ui/AGENTS.md).

```bash
cd apps/ui && npm install && npm run dev
```

## Data Directory

All apps use `~/Documents/zbot/` by default:

```
zbot/
├── config/
│   ├── settings.json          # App settings
│   ├── providers.json         # LLM provider configurations
│   ├── mcp-servers.json       # MCP server configurations
│   ├── connectors.json        # External connectors
│   ├── schedules.json         # Scheduled tasks
│   ├── agent/                 # User-editable agent contracts
│   │   ├── SOUL.md
│   │   ├── INSTRUCTIONS.md
│   │   └── OS.md
│   ├── agent-prompts/         # User-editable prompt modules
│   └── auth/mcp/              # OAuth pending state and tokens
├── data/
│   └── conversations.db       # SQLite runtime conversation data
├── agents/{name}/        # Agent configs
├── wards/                # Code project directories
├── skills/{name}/        # Skill definitions
├── plugins/              # Created only when a plugin is installed
└── temp/                 # Created only when transient work is needed
```
