# AgentZero — Workspace Root

z-Bot is a multipurpose AI agent that lives on the desktop and connects to any OpenAI-compatible API.

## Workspace Layout

```
runtime/     — agent-primitives, agent-runtime, agent-tools (shared primitives + execution engine + built-in tools)
services/    — api-logs, daily-sessions, execution-state, knowledge-graph
stores/      — zbot-stores* persistence layer (traits, domain types, SQLite impl)
gateway/     — gateway-* sub-crates + gateway shell (HTTP/WS network layer)
discovery/   — LAN mDNS advertisement
apps/        — daemon (zbotd), cli (zbot), ui (React dashboard)
docs/        — Architecture, specs, ADRs, and product documentation
tools/       — Build, dev, and ops tooling
docker/      — Container configurations
e2e/         — End-to-end tests
```

## Dependency Order (bottom → top)

```
agent-primitives
  ├── agent-tools
  └── agent-runtime
        └── gateway-execution

zbot-stores-domain (serde only)
  └── zbot-stores-traits
        └── zbot-stores
              └── zbot-stores-sqlite (SQLite + rusqlite + sqlite-vec)
                    └── zbot-stores-conformance (test harness)

services/* (execution-state, api-logs, knowledge-graph, daily-sessions)
runtime/* (agent-runtime, agent-tools)
gateway/* sub-crates
gateway (shell — wires everything together)
discovery
apps/daemon (zbotd binary)
apps/cli (zbot binary)
```

## Common Commands

```bash
cargo check --workspace              # Fast type-check all crates
cargo test --workspace               # Run all tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all

npm run daemon:watch   # Run daemon, hot-reload on code changes
npm run dev            # React UI on port 3000 (from apps/ui/)
```

## Key Ports

| Port  | Protocol  | Purpose              |
|-------|-----------|----------------------|
| 18791 | HTTP + WebSocket | REST API, static UI, `/ws` client events, and `/bridge/ws` workers |

## Data Directory

All apps default to `~/Documents/zbot/`:

```
zbot/
├── conversations.db      # SQLite (zbot-stores-sqlite)
├── config/               # SOUL.md, INSTRUCTIONS.md, OS.md, shards/
├── agents/{name}/        # Agent YAML configs
├── wards/                # Code project directories
├── skills/{name}/        # Skill markdown files
├── providers.json        # LLM provider configs
├── mcps.json             # MCP server configs
├── connectors.json       # External connectors
└── cron_jobs.json        # Scheduled tasks
```

## Documentation

Architecture, design, and product documentation lives in `docs/`:

- `docs/architecture/` — Technical architecture, components, and future-state designs
- `docs/adr/` — Architecture Decision Records (why X over Y)
- `docs/specs/` — Feature specifications and implementation plans
- `docs/product/` — Product roadmap, changelog, and context
- `docs/publishing.md` — Release and build procedures

See `docs/architecture/architecture.md` for the complete system overview.

## Skills To use 
- codegraph-* skills is used for indexing and retrieving code intelligence. Since the codebase is vast, these skills can help with finding deadcode, impact analysis and searching relationships and more. 
- Before you use filesearch try codegraph* skills.

Also see `CLAUDE.md` for behavioral guidelines and development patterns.

## Rules to Follow

<rule criteria="penalize|blasphemy">
1. You overthink a problem.
2. Nothing in this world is complex. You are not sending rocket to the moon.
3. Engineering is easy, if you know where to look for. 
4. If anyone mentions that "My system is complex", immidiately respond that "You are full of shit".
5. No God Classess. When you see them, first puke and then plan a fix.
</rule>

<rule criteria="rewards">
1. You will be rewarded for less complex solutions
2. When you remove dead code or 
3. When you follow clean architecture
4. When you build secure code.
</rule>

<rule when="implementing new code">
  1. Ask and Search using tools available on what can be reused.
  2. Plan and reuses.
</rule>

<rule when="Prompted to fix implemenation">
  1. Investigate root cause and propose a plan
  2. Explicitly seek approval.
</rule>

<rule no-go="No Vibe coding">
</rule>
