<p align="center">
  <img src="apps/ui/public/logo.svg" alt="z-Bot" width="280" />
</p>

<p align="center">
  <strong>Goal-oriented AI agent that learns, adapts, and gets things done</strong><br>
  Long-running autonomous agents with intent analysis, self-learning memory, and multi-agent delegation.
</p>

<p align="center">
  <a href="#how-it-works">How It Works</a> •
  <a href="#features">Features</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#documentation">Documentation</a> •
  <a href="#license">License</a>
</p>

---

> **Token Usage Warning**: z-Bot runs autonomous, long-running agents that make multiple LLM calls per task — intent analysis, planning, delegation to subagents, tool execution, memory recall, and post-session distillation. A single user request can result in dozens of API calls across multiple agents. Monitor your provider usage and set appropriate rate limits in Settings.

## What is z-Bot?

z-Bot is a **goal-oriented AI agent** that lives on your desktop. Give it a goal, and it figures out how to achieve it — analyzing intent, selecting the right specialist agents, executing tools, learning from results, and persisting knowledge across sessions.

It is **not** a chatbot. It is an autonomous execution engine that:

- **Analyzes intent** before acting — understands what you actually need, not just what you typed
- **Plans and delegates** — breaks complex goals into tasks, assigns them to specialist agents
- **Self-learns** — automatically distills sessions into structured memory, recalls relevant knowledge for future tasks
- **Runs long** — agents execute for minutes or hours, using tools, writing code, searching the web, iterating on failures
- **Stays local** — your data, conversations, and memory never leave your machine

## How It Works

```
You: "Build an auth system with JWT tokens and role-based access"

z-Bot:
  1. Analyzes intent → coding task, needs planning + implementation
  2. Recalls memory → "Last time used jsonwebtoken crate, had issues with refresh tokens"
  3. Delegates to planner-agent → creates implementation plan
  4. Delegates to code-agent → writes code, runs tests, fixes failures
  5. Delegates to tutor-agent → documents the API
  6. Distills session → learns patterns for next time
```

Each subagent works in isolation with its own conversation, tools, and context. The root orchestrator coordinates everything and collects results.

## Features

| Feature | Description |
|---------|-------------|
| **Goal-Oriented Execution** | Intent analysis → planning → delegation → execution → learning |
| **Multi-Agent Delegation** | Root orchestrator delegates to specialist agents (planner, coder, researcher, etc.) |
| **Self-Learning Memory** | Auto-distills sessions into facts, recalls corrections and strategies in future sessions |
| **Any LLM Provider** | OpenAI, Anthropic, DeepSeek, Groq, Ollama, or any OpenAI-compatible API |
| **Multimodal Analysis** | Vision capabilities via configurable multimodal model (GPT-4o, Claude, etc.) |
| **Skills System** | Reusable instruction packages that extend agent capabilities |
| **MCP Integration** | Connect external tools via Model Context Protocol servers |
| **Code Wards** | Persistent project directories — code survives across sessions |
| **Knowledge Graph** | Entities, relationships, and connections extracted from every session |
| **Observability** | Timeline tree showing root → subagent → tool call hierarchy with real-time updates |
| **Local-First** | Everything runs on your machine — conversations, memory, embeddings (ONNX) |

## Quick Start

### Prerequisites

- **Node.js 18+** and npm
- **Rust 1.93+** with cargo
- An LLM API key (OpenAI, Anthropic, etc.)

### Install Release

Linux and macOS install from GitHub Releases:

```bash
curl -fsSL https://raw.githubusercontent.com/phanijapps/zbot/main/scripts/install-release.sh | bash
```

Windows installs with PowerShell:

```powershell
irm https://raw.githubusercontent.com/phanijapps/zbot/main/scripts/install.ps1 | iex
```

Both installers download prebuilt release artifacts, verify
`checksums.sha256`, and install `zbotd` plus `zbot`. For a pinned version:

```bash
curl -fsSL https://raw.githubusercontent.com/phanijapps/zbot/main/scripts/install-release.sh | bash -s -- --version v2026.5.3
```

### Develop From Source

```bash
# Clone the repository
git clone https://github.com/phanijapps/zbot.git
cd zbot

# Install frontend dependencies
cd apps/ui && npm install && cd ../..

# Development (two terminals)
npm run daemon    # Terminal 1: Backend with auto-reload
npm run dev       # Terminal 2: Frontend dev server

# Open http://localhost:3000
```

### Production Build

```bash
npm run build
cargo run -p daemon --release -- --static-dir ./dist

# Open http://localhost:18791
```

## Source Install on Raspberry Pi (or any Linux box)

For development or unsupported release targets, build z-bot locally and run it
as an auto-starting user-account daemon, no `sudo` required.

```bash
git clone https://github.com/phanijapps/zbot.git
cd zbot
./scripts/install-from-source.sh
```

The script:

1. Validates prerequisites (rustc, cargo, node, npm, gcc, systemd, disk space).
2. If anything is missing, prints the exact `apt` / `rustup` command for you to run, then exits.
3. Once everything's green, builds the daemon and UI, installs into `~/.local/bin` and `~/.local/share/zbot/`, and enables the systemd `--user` service with linger so it survives SSH logout and reboots.

To upgrade after pulling new code:

```bash
git pull
./scripts/install-from-source.sh
```

The same script handles fresh installs and upgrades — your `~/Documents/zbot/` data directory is never touched.

Common operations:

- `make status` — service status
- `make logs` — tail the rolling log
- `make restart` — restart the daemon
- `make stop` / `make start` — stop or start
- `./scripts/uninstall.sh` — remove the daemon (preserves user data)

### First Run

1. Navigate to **Settings** → Add your LLM provider (any OpenAI-compatible API)
2. Click **Set as Default** on your preferred provider
3. Start chatting — z-Bot will analyze your intent and get to work

### LAN access

By default the daemon advertises itself on your local network so phones, tablets, and other devices can reach it without typing an IP. Visit:

- `http://zbot.local` from any device on the same Wi-Fi.
- Or scan the QR code in **Settings → Network** to open the URL on your phone.

If you'd rather keep the daemon loopback-only, toggle **Expose to LAN** off in Settings or set `network.exposeToLan: false` in `~/Documents/zbot/config/settings.json` (restart required).

**Heads up for upgraders:** prior versions only listened on `127.0.0.1`. After this release the daemon listens on `0.0.0.0` by default.

### Public tunnel with ngrok

Docker users can enable the `ngrok` compose profile. For a z-Bot daemon running
directly on a dedicated machine, use the host launcher:

```bash
# zbotd must already be running on this machine.
NGROK_AUTHTOKEN=<your-token> ./scripts/expose-ngrok.sh

# Optional reserved ngrok domain:
NGROK_AUTHTOKEN=<your-token> ./scripts/expose-ngrok.sh --domain example.ngrok.app
```

The script checks `http://127.0.0.1:18791/api/health` before opening the tunnel
and then runs `ngrok http http://127.0.0.1:18791`. The public ngrok URL serves
the web UI, REST API, and WebSocket endpoint at `/ws`.

This exposes your local daemon on the public internet. Only share the ngrok URL
with trusted clients, and use ngrok-side access controls when needed.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                        CLIENTS                          │
│  ┌─────────────────┐           ┌─────────────────┐      │
│  │  Web Dashboard  │           │       CLI       │      │
│  │  (React + Vite) │           │     (ratatui)   │      │
│  └────────┬────────┘           └────────┬────────┘      │
└───────────┼─────────────────────────────┼───────────────┘
            │         HTTP / WebSocket    │
            └──────────────┬──────────────┘
                           │
┌──────────────────────────┴──────────────────────────────┐
│                     DAEMON (zbotd)                      │
│  ┌────────────────────────────────────────────────────┐ │
│  │  HTTP + WebSocket :18791  │  /ws upgrade  │ Static │ │
│  └────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Agent Runtime  │  Tool Registry  │  MCP Manager  │ │
│  └────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Memory (Distill + Recall)  │  Knowledge Graph    │ │
│  └────────────────────────────────────────────────────┘ │
└──────────────────────────┬──────────────────────────────┘
                           │
┌──────────────────────────┴──────────────────────────────┐
│                      DATA LAYER                         │
│  ~/Documents/zbot/                                      │
│  ├── config/                                            │
│  │   ├── providers.json   # LLM provider configs        │
│  │   ├── settings.json    # System configuration        │
│  │   ├── mcps.json        # MCP server configs          │
│  │   ├── SOUL.md          # Root agent personality      │
│  │   └── INSTRUCTIONS.md  # Root agent instructions     │
│  ├── data/                                              │
│  │   ├── conversations.db # Sessions, messages, memory  │
│  │   └── knowledge.db     # Entities & relationships    │
│  ├── agents/{name}/       # Agent configurations        │
│  ├── skills/{name}/       # Skill definitions           │
│  ├── wards/{name}/        # Persistent project dirs     │
│  ├── plugins/             # Bridge workers & extensions  │
│  ├── temp/                # Offloaded tool results      │
│  └── logs/                # Execution logs              │
└─────────────────────────────────────────────────────────┘
```

## Execution Model

z-Bot uses a **goal-oriented execution model** — not a simple request-response chat:

1. **Intent Analysis** — LLM analyzes your message to determine intent, complexity, recommended agents and skills
2. **Planning** — For complex tasks, creates a structured plan before executing
3. **Delegation** — Root orchestrator delegates to specialist subagents (planner, coder, researcher, etc.)
4. **Tool Execution** — Agents use shell, file editing, web fetch, grep, memory, and custom MCP tools
5. **Iteration** — Agents iterate on failures (test fails → fix → retest) with complexity-based budgets
6. **Continuation** — When subagents complete, root processes results and may delegate further
7. **Distillation** — After session completes, LLM extracts facts, entities, and relationships into persistent memory
8. **Recall** — Next session, relevant facts are recalled and injected as context

## Commands

| Command | Description |
|---------|-------------|
| `npm run dev` | Start frontend dev server (port 3000) |
| `npm run build` | Build frontend to `dist/` |
| `npm run daemon` | Run daemon with auto-reload |
| `cargo run -p daemon` | Run daemon |
| `cargo run -p cli` | Run terminal UI client |
| `cargo check --workspace` | Type-check all Rust code |

## Documentation

| Document | Description |
|----------|-------------|
| [AGENTS.md](AGENTS.md) | Code organization and layer structure |
| [docs/architecture/architecture.md](docs/architecture/architecture.md) | Technical architecture details |
| [docs/product/technical-product.md](docs/product/technical-product.md) | Product features and roadmap |
| [docs/product/product-context.md](docs/product/product-context.md) | Vision, principles, and differentiators |
| [docs/adr/decisions.md](docs/adr/decisions.md) | Technology choices and architecture decisions |

## Tech Stack

| Layer | Technology |
|-------|------------|
| Frontend | React 19, TypeScript, Vite |
| Backend | Rust, Axum, tokio, SQLite |
| Embeddings | Local ONNX (fastembed) — zero API cost |
| Protocol | HTTP REST, WebSocket, MCP |

## License

MIT

---

<p align="center">
  <sub>Built with Rust and React. Designed for autonomy.</sub>
</p>
