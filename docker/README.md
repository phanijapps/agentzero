# z-Bot Docker

Run z-Bot in Docker with a single command. The agent runs in a sandboxed container with access only to your vault directory.

## Quick Start

```bash
cd docker/
./start.sh
```

First build takes ~10 minutes (Rust compilation). Subsequent starts are instant.
The launcher creates `~/zbot` on the host before Docker starts so the vault is
owned by your host user, not by root.

Open **http://localhost:18791** and complete setup.

## Manual Setup

```bash
cd docker/
cp .env.example .env
# Edit .env — set VAULT_PATH and ports as needed
mkdir -p ~/zbot
chown "$(id -u):$(id -g)" ~/zbot
docker compose up -d --build
```

Open the app at `http://localhost:${HTTP_PORT}`. With the default `.env.example`,
that is `http://localhost:18791`.

## Configuration

Edit `docker/.env`:

| Variable | Default | Description |
|----------|---------|-------------|
| `VAULT_PATH` | `~/zbot` | Host directory for all z-Bot data |
| `HTTP_PORT` | `18791` | Web UI + HTTP API + `/ws` WebSocket upgrade |
| `NGROK_AUTHTOKEN` | empty | Optional ngrok auth token for public tunnel exposure |
| `NGROK_DOMAIN` | empty | Optional reserved ngrok domain, for example `example.ngrok.app` |
| `NGROK_WEB_PORT` | `4040` | Host port for the local ngrok inspector UI |

## Expose with ngrok

The compose file includes an optional `ngrok` profile. Keep your token in
`docker/.env`; do not commit it.

```bash
cd docker/
cp .env.example .env
# Edit .env and set NGROK_AUTHTOKEN
docker compose --profile ngrok up -d --build
```

The ngrok container tunnels the internal app URL `http://zbot:18791`.

After startup:

- Local app: `http://localhost:${HTTP_PORT}`
- Local ngrok inspector: `http://localhost:${NGROK_WEB_PORT}`
- Public URL: shown in the ngrok inspector or `docker compose logs ngrok`

If you have a reserved ngrok domain, set `NGROK_DOMAIN=your-domain.ngrok.app`
before starting the profile. If `NGROK_DOMAIN` is empty, ngrok assigns a
temporary public URL.

## What's in the Container

| Tool | Version | Purpose |
|------|---------|---------|
| python3 | System | Agent scripts, Python wards |
| uv | Latest | Fast Python package management |
| node | 20 | Agent scripts, Node wards |
| pnpm | Latest | Fast Node package management |
| libreoffice | System | Document conversion (soffice) |
| git | System | Version control in wards |

## Vault Directory

The vault (`VAULT_PATH`) is bind-mounted into the container. Create the host
directory before starting Compose. If the path does not exist, Docker may create
it as `root`, which prevents the non-root `zbot` container user from writing to
`/data/zbot`.

The daemon creates the full directory structure inside the vault on first run:

```
zbot/
├── config/          # providers.json, settings.json, mcps.json, cron_jobs.json, …
├── data/            # conversations.db, knowledge.db
├── agents/          # Agent configurations
├── agents_data/     # Per-agent runtime data + shared memory
├── skills/          # Skill definitions
├── wards/           # Persistent project directories
├── plugins/         # Bridge workers
├── logs/            # Execution logs
└── temp/            # Ephemeral scratch (auto-wiped)
```

## Embedding Models

ONNX embedding models (~100MB) are downloaded on first use and cached in a Docker volume (`fastembed-cache`). They persist across container restarts.

## Sandbox

The container can only access:
- The mounted vault directory
- The fastembed cache volume
- Network (for LLM API calls)

It **cannot** access any other host files, processes, or resources.

## Commands

```bash
# Start
docker compose up -d

# Stop
docker compose down

# View logs
docker compose logs -f

# Rebuild after code changes
docker compose up -d --build

# Full reset (removes embedding cache)
docker compose down -v
```
