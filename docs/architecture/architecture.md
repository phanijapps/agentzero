# z-Bot — Technical Architecture

## System Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           CLIENTS                                        │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────┐       ┌─────────────────────────┐          │
│  │     Web Dashboard       │       │          CLI            │          │
│  │    (React + Vite)       │       │        (zbot)           │          │
│  │    localhost:3000       │       │                         │          │
│  └───────────┬─────────────┘       └───────────┬─────────────┘          │
│              │ HTTP/WebSocket                   │ HTTP/WebSocket         │
└──────────────┼──────────────────────────────────┼────────────────────────┘
               │                                  │
               └────────────────┬─────────────────┘
                                │
┌───────────────────────────────┴─────────────────────────────────────────┐
│                           DAEMON (zbotd)                                 │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                         GATEWAY                                  │    │
│  │  ┌──────────────────────────┐   ┌─────────────┐                  │    │
│  │  │   HTTP + WebSocket       │   │   Static    │                  │    │
│  │  │   :18791                 │   │   Files     │                  │    │
│  │  │   (Axum + axum-ws)       │   │  (tower)    │                  │    │
│  │  │   `/ws` upgrade route    │   │             │                  │    │
│  │  └──────────────┬───────────┘   └─────────────┘                  │    │
│  │                 │                                                 │    │
│  │                 │   (legacy standalone WS :18790, off by default; │    │
│  │                 │    flip `legacy_ws_port_enabled` to bind it)    │    │
│  │                  │                                               │    │
│  │         ┌────────┴────────┐                                      │    │
│  │         │    Event Bus    │ ◄─── Broadcast streaming events      │    │
│  │         └────────┬────────┘                                      │    │
│  └──────────────────┼───────────────────────────────────────────────┘    │
│                     │                                                    │
│  ┌──────────────────┴───────────────────────────────────────────────┐    │
│  │                      AGENT RUNTIME                                │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │    │
│  │  │  Executor   │  │ LLM Client  │  │    Tool     │              │    │
│  │  │   (loop)    │──│  (per-task  │  │  Registry   │              │    │
│  │  │  +stuck det │  │   routed)   │  │             │              │    │
│  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘              │    │
│  │         │                │                │                     │    │
│  │         │       ┌────────┴────────┐       │                     │    │
│  │         └───────│  MCP / Plugins  │───────┘                     │    │
│  │                 └─────────────────┘                             │    │
│  │                                                                 │    │
│  │  ┌──────────────────────────────────────────────────────────┐  │    │
│  │  │           COGNITIVE SUBSTRATE                             │  │    │
│  │  │ Wards (delegatable agents)  •  Ward Curator              │  │    │
│  │  │ Memory Brain (facts, episodes, beliefs, procedures,      │  │    │
│  │  │   knowledge graph, hierarchy, bi-temporal)               │  │    │
│  │  │ Sleep-time pipeline (synth, beliefs, abstraction)        │  │    │
│  │  │ Intent Analysis  •  Resource Indexer                     │  │    │
│  │  │ Agent Pool (wait/kill/steer running subagents)           │  │    │
│  │  └──────────────────────────────────────────────────────────┘  │    │
│  └──────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                         DATA LAYER                                       │
├─────────────────────────────────────────────────────────────────────────┤
│  ~/Documents/zbot/                  (vault root; --vault overrides)      │
│  ├── config/                          # App config, prompts, registries  │
│  │   ├── settings.json                #   App settings (network, logs)   │
│  │   ├── providers.json               #   LLM provider credentials       │
│  │   ├── recall_config.json           #   Recall tuning: weights, graph  │
│  │   ├── mcps.json                    #   MCP server configurations      │
│  │   ├── connectors.json              #   Connector configurations       │
│  │   ├── cron_jobs.json               #   Scheduled job configurations   │
│  │   ├── seeded_defaults.json         #   IDs of bundled defaults seeded │
│  │   ├── SOUL.md                      #   Agent identity/personality     │
│  │   ├── INSTRUCTIONS.md              #   Execution rules                │
│  │   ├── OS.md                        #   Platform commands (auto-gen)   │
│  │   ├── distillation_prompt.md       #   Distillation prompt override   │
│  │   ├── intent_analysis_prompt.md    #   Intent-analysis prompt override│
│  │   ├── shards/                      #   Overridable prompt shards      │
│  │   │   ├── tooling_skills.md        #     Skills-first approach        │
│  │   │   ├── memory_learning.md       #     Memory patterns              │
│  │   │   └── planning_autonomy.md     #     Planning and autonomy        │
│  │   └── wards/                       #   Per-language ward index configs│
│  │       └── *.yaml                                                      │
│  ├── data/                            # SQLite databases                  │
│  │   ├── conversations.db             #   Conversations, messages,       │
│  │   │                                #   memory_facts, embedding_cache  │
│  │   └── knowledge.db                 #   Knowledge graph + vec0 indexes │
│  ├── logs/                            # Daemon log files (when enabled)  │
│  │   └── zbotd.YYYY-MM-DD.log         #   Rolling log files              │
│  ├── agents/{name}/                   # Agent configurations             │
│  │   ├── config.yaml                  #   Model, provider, temperature   │
│  │   └── AGENTS.md                    #   System instructions            │
│  ├── agents_data/{id}/                # Per-agent runtime data           │
│  │   └── memory.json                  #   Persistent key-value storage   │
│  ├── agents_data/shared/              # Cross-agent shared memory        │
│  │   ├── user_info.json               #   User preferences               │
│  │   ├── workspace.json               #   Project paths (auto-injected)  │
│  │   ├── patterns.json                #   Learned patterns               │
│  │   └── session_summaries.json       #   Distilled learnings            │
│  ├── skills/{name}/                   # Vault-owned skill definitions    │
│  │   └── SKILL.md                     #   Instructions + frontmatter     │
│  ├── wards/                           # Wards (delegatable agents + dirs)│
│  │   ├── .venv/                       #   Shared Python venv             │
│  │   ├── .node_env/                   #   Shared Node tooling            │
│  │   ├── scratch/                     #   Default ward for quick tasks   │
│  │   └── {ward-name}/                 #   Domain-scoped specialist       │
│  │       ├── AGENTS.md                #     Doctrine: scope + mandate +  │
│  │       │                            #       runtime hints (no ZBOT.md) │
│  │       ├── config.yaml              #     Per-ward LLM override        │
│  │       ├── memory-bank/             #     Curated knowledge            │
│  │       │   ├── ward.md              #       Rules (corrections/etc.)   │
│  │       │   ├── core_docs.md         #       Function signatures        │
│  │       │   └── structure.md         #       Directory tree             │
│  │       └── (project files)          #     Code, reports, artifacts     │
│  ├── plugins/                         # Node.js plugin directories       │
│  │   ├── .example/                    #   Reference plugin               │
│  │   └── {plugin-name}/                                                  │
│  │       ├── plugin.json              #     Plugin manifest              │
│  │       ├── package.json             #     Node.js dependencies         │
│  │       ├── index.js                 #     Entry point                  │
│  │       ├── .config.json             #     User config + secrets        │
│  │       └── node_modules/            #     Auto-installed deps          │
│  └── temp/                            # Ephemeral scratch (auto-wiped)   │
└─────────────────────────────────────────────────────────────────────────┘
```

> **Vault-path source of truth:** `gateway/gateway-services/src/paths.rs`. Schema for `config/*.json` files lives in the consuming services (e.g. `gateway-services::SettingsService`, `gateway-cron::CronJobsStore`). System-wide skills also load from `~/.agents/skills/` outside the vault.
>
## Technology Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Frontend | React 19 + TypeScript | UI components |
| Build | Vite | Fast dev server, bundling |
| UI | Tailwind CSS v4 + Radix UI | Styling, accessible primitives |
| HTTP Server | Axum | Async HTTP framework |
| WebSocket | tokio-tungstenite | Real-time streaming |
| Async Runtime | tokio | Async I/O |
| Database | SQLite (rusqlite + r2d2 pool) | Conversations, memory facts, embeddings (WAL mode) |
| Embeddings | fastembed (local ONNX) | Default: all-MiniLM-L6-v2 (384d), zero cost |
| Serialization | serde + serde_json | JSON handling |
| Logging | tracing + tracing-subscriber + tracing-appender | Structured logging with file rotation |

## Model Configuration

Models are configured directly on providers, agents, and Advanced settings.
There is no bundled or local `models.json` catalog. Unknown/custom model names
use the fallback token limits of 200k input and 32k output unless provider,
agent, or Advanced settings override them.

### ModelProfile Structure

```json
{
  "glm-5.1": {
    "name": "GLM-5.1",
    "provider": "zhipu",
    "capabilities": {
      "tools": true, "vision": true, "thinking": true,
      "embeddings": false, "voice": false,
      "imageGeneration": false, "videoGeneration": false
    },
    "context": { "input": 128000, "output": 16384 }
  }
}
```

### Capabilities

| Capability | Description |
|------------|-------------|
| `tools` | Function/tool calling support |
| `vision` | Image input support |
| `thinking` | Extended reasoning/chain-of-thought |
| `embeddings` | Vector embedding model (not chat) |
| `voice` | Audio input/output |
| `imageGeneration` | Image generation (DALL-E style) |
| `videoGeneration` | Video generation/processing |

### How It's Used

- **ExecutorBuilder**: Resolves context window from registry (replaces hardcoded lookup). Validates `thinking_enabled` against model capabilities.
- **Delegation spawn**: Validates subagent model supports `tools` capability.
- **UI**: Model dropdowns show capability badges (wrench, eye, brain, speaker) with tooltips.
- **API**: `GET /api/models` returns the full merged registry.

### Implementation Files

| File | Purpose |
|------|---------|
| `gateway/gateway-services/src/models.rs` | Fallback model profile and default token constants |
| `gateway/src/http/models.rs` | REST API endpoints |
| `gateway/gateway-execution/src/invoke/executor.rs` | Effective token limit resolution |

## Logging Configuration

z-Bot supports configurable file logging with automatic rotation and retention management. Logging can be configured via `settings.json` or CLI arguments.

### Configuration Sources

| Source | Priority | Persistence |
|--------|----------|-------------|
| CLI arguments | Highest | Session only |
| `settings.json` | Medium | Persistent |
| Defaults | Lowest | N/A |

### LogSettings Structure

```json
{
  "logs": {
    "enabled": false,
    "directory": null,
    "level": "info",
    "rotation": "daily",
    "maxFiles": 7,
    "suppressStdout": false
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Enable file logging |
| `directory` | string\|null | `{data_dir}/logs` | Custom log directory |
| `level` | string | `"info"` | Log level: `trace`, `debug`, `info`, `warn`, `error` |
| `rotation` | string | `"daily"` | Rotation: `daily`, `hourly`, `minutely`, `never` |
| `maxFiles` | number | `7` | Max rotated files to keep (0 = unlimited) |
| `suppressStdout` | bool | `false` | Only log to file (daemon mode) |

### CLI Arguments

```bash
# Enable file logging with custom directory
zbotd --log-dir /var/log/zbot

# Configure rotation and retention
zbotd --log-dir ./logs --log-rotation hourly --log-max-files 24

# Daemon mode (file only, no stdout)
zbotd --log-dir ./logs --log-no-stdout

# Set log level
zbotd --log-level debug
```

### Log File Location

| Platform | Default Location |
|----------|-----------------|
| Windows | `C:\Users\{user}\Documents\zbot\logs\` |
| macOS | `/Users/{user}/Documents\zbot/logs/` |
| Linux | `/home/{user}/Documents/zbot/logs/` |

### Log File Naming

```
{data_dir}/logs/
├── zbotd.2024-02-14.log      # Current (daily rotation)
├── zbotd.2024-02-13.log      # Rotated yesterday
├── zbotd.2024-02-12.log      # Rotated 2 days ago
└── ...                        # Older logs (deleted when > maxFiles)
```

### HTTP API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/settings/logs` | Get current log settings |
| PUT | `/api/settings/logs` | Update log settings (requires restart) |

**Note:** Changes to log settings via the API require a daemon restart to take effect.

### Implementation Files

| File | Purpose |
|------|---------|
| `gateway/gateway-services/src/logging.rs` | `LogSettings` struct with validation |
| `gateway/gateway-services/src/settings.rs` | `AppSettings` with `logs` field, CRUD methods |
| `gateway/src/http/settings.rs` | HTTP endpoints for log settings |
| `apps/daemon/src/main.rs` | Logging initialization with settings.json + CLI merge |
| `apps/ui/src/features/settings/WebSettingsPanel.tsx` | Settings page (context protection, logging, advanced) |

## Execution Settings

Controls agent concurrency and first-time setup state. Stored in `settings.json` under `execution`.

### ExecutionSettings Structure

```json
{
  "execution": {
    "maxParallelAgents": 2,
    "setupComplete": false,
    "agentName": "Brahmi",
    "orchestrator": {
      "providerId": null,
      "model": null,
      "temperature": 0.7,
      "maxTokens": 16384,
      "thinkingEnabled": true
    },
    "distillation":   { "providerId": null, "model": null },
    "curator":        { "providerId": null, "model": null },
    "intentAnalysis": { "providerId": null, "model": null },
    "sleepTime":      { "providerId": null, "model": null },
    "multimodal": {
      "providerId": null,
      "model": null,
      "temperature": 0.3,
      "maxTokens": 4096
    }
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `maxParallelAgents` | number | `2` | Max subagents running concurrently (global semaphore) |
| `setupComplete` | bool | `false` | Whether setup wizard has been completed |
| `agentName` | string\|null | `null` | User-chosen root agent name (also written to SOUL.md) |
| `orchestrator.providerId` | string\|null | `null` | Provider for root agent (null = default provider) |
| `orchestrator.model` | string\|null | `null` | Model for root agent (null = provider's default) |
| `orchestrator.temperature` | number | `0.7` | Temperature (0-2) |
| `orchestrator.maxTokens` | number | `16384` | Max output tokens (higher for thinking) |
| `orchestrator.thinkingEnabled` | bool | `true` | Extended reasoning before delegating |
| `distillation.{providerId,model}` | string\|null | `null` | Post-session fact extraction override |
| `curator.{providerId,model}` | string\|null | `null` | Ward consolidation cycle override |
| `intentAnalysis.{providerId,model}` | string\|null | `null` | Per-prompt routing classifier override |
| `sleepTime.{providerId,model}` | string\|null | `null` | Memory-cycle pipeline override (synthesis, beliefs, abstraction, contradiction detection, verifier, handoff) |
| `multimodal.providerId` | string\|null | `null` | Provider for default vision model |
| `multimodal.model` | string\|null | `null` | Vision-capable model (e.g., GPT-4o, gemma4) |
| `multimodal.temperature` | number | `0.3` | Temperature for analysis (lower = deterministic) |
| `multimodal.maxTokens` | number | `4096` | Max output tokens for vision responses |

All non-orchestrator slots inherit from `orchestrator` when their `providerId` / `model` are `null`. See **Per-Task LLM Routing** below for the resolution mechanics.

### Per-Task LLM Routing

Different stages of the agent loop have different cost / latency / capability tradeoffs. Each role routes through a 3-tier resolution chain:

```
   call site                                            chosen provider+model
       │
       ▼
 LlmClientConfig::new(temp, tokens)                ┌────────────────────────┐
   .with_task("sleep_time")  ───────────────►      │  per-task override     │
                                                   │  (settings.execution.  │
                                                   │   sleepTime/curator/   │
                                                   │   intentAnalysis/...)  │
                                                   └──────────┬─────────────┘
                                                              │ none?
                                                              ▼
                                                   ┌────────────────────────┐
                                                   │  orchestrator override │
                                                   │  (settings.execution.  │
                                                   │   orchestrator)        │
                                                   └──────────┬─────────────┘
                                                              │ none?
                                                              ▼
                                                   ┌────────────────────────┐
                                                   │  provider default      │
                                                   │  (providers.json)      │
                                                   └────────────────────────┘
```

Mechanism:
- `LlmClientConfig` carries an optional `task: Option<String>` tag, set via the `.with_task(...)` builder
- `ProviderServiceLlmFactory::build_client` (in `gateway/src/memory_llm_factory.rs`) reads `ExecutionSettings` at call time and picks the right slot per-tag
- Untagged callers (chat, recall) fall through to the orchestrator silently

Currently-tagged tasks:

| Tag | Call sites | Settings slot |
|-----|-----------|---------------|
| `sleep_time` | `synthesizer`, `belief_synthesizer`, `corrections_abstractor`, `verifier`, `pattern_extractor`, `conflict_resolver`, `llm_aggregate_entity`, `belief_contradiction_detector`, `handoff_writer` | `execution.sleepTime` |
| `curator` | Phase C ward consolidation (`gateway/src/ward_curator.rs`) | `execution.curator` |
| `intent_analysis` | Per-prompt classifier | `execution.intentAnalysis` |
| `distillation` | `SessionDistiller` | `execution.distillation` |
| `multimodal` | `multimodal_analyze` tool | `execution.multimodal` |

Per-ward LLM config (`wards/{name}/config.yaml`) is a separate override that applies only when the ward is delegated to — it does not flow through the task tag.

### HTTP API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/settings/execution` | Get execution settings |
| PUT | `/api/settings/execution` | Update (also writes SOUL.md if agentName changes) |
| GET | `/api/setup/status` | Lightweight check: `{ setupComplete, hasProviders }` |
| GET | `/api/setup/mcp-defaults` | Sanitized MCP templates for wizard |

## Provider Management

Providers are LLM API connections stored in `providers.json`. Each provider has rate limits, optional per-model configs, and a verified status.

### Rate Limiting

Default: 30 RPM / 2 concurrent (configurable per provider). Shared across all executors using the same provider via `ProviderRateLimiter`.

```
OpenAiClient → RetryingLlmClient → RateLimitedLlmClient
```

| Layer | Purpose | File |
|-------|---------|------|
| `OpenAiClient` | Raw OpenAI-compatible API calls | `runtime/agent-runtime/src/llm/openai.rs` |
| `RetryingLlmClient` | Automatic retry on transient errors | `runtime/agent-runtime/src/llm/retry.rs` |
| `RateLimitedLlmClient` | Per-provider semaphore + sliding window RPM | `runtime/agent-runtime/src/llm/rate_limiter.rs` |

### Model Enrichment

API responses (`GET /api/providers`) inject default rate limits and may expose fallback model metadata for listed models. Provider `modelConfigs`, agent settings, and Advanced settings are the supported places for token overrides.

## LLM Client — Text and Multimodal Content

The LLM client layer supports multimodal content (images, files) alongside text. Content flows through typed `Part` variants and gets encoded to OpenAI-compatible format at the provider boundary.

### Content Model

Messages carry content as `Vec<Part>` where each Part is one of:

| Part | Fields | Wire Format |
|------|--------|-------------|
| `Text` | `text: String` | `{ "type": "text", "text": "..." }` |
| `Image` | `source: ContentSource, mime_type, detail` | `{ "type": "image_url", "image_url": { "url": "data:...;base64,...", "detail": "high" } }` |
| `File` | `source: ContentSource, mime_type, filename` | `{ "type": "file", "file": { "url": "data:...;base64,..." } }` |
| `FunctionCall` | `name, args, id` | Handled separately as tool_calls |
| `FunctionResponse` | `id, response` | Handled separately as tool results |

`ContentSource` has three variants:
- `Url(String)` — remote URL, stored as-is
- `Base64(String)` — inline encoded bytes, ephemeral (never persisted to DB)
- `FileRef(String)` — local file path, what DB stores after flushing Base64 to disk

### Backward Compatibility

`ChatMessage` has custom serde: text-only messages serialize `content` as a plain string (backward compat with all providers), multimodal messages serialize as a content array. Deserialization accepts both formats.

### Provider Encoding

`OpenAiClient` performs OpenAI-compatible request encoding directly. Capability-aware encoding rejects Image/File parts for non-vision models and surfaces an execution error so the planning agent can route to a vision-capable model.

### Content Persistence

Base64 blobs are flushed to content-addressed files (SHA-256 hash) before DB persistence. On rehydration (before LLM call), `FileRef` sources are read from disk and re-encoded to Base64.

```
Inbound: Part::Image { Base64("...") }
  → flush_part_to_disk() → Part::Image { FileRef("/attachments/abc123.png") }
  → DB stores FileRef
  → rehydrate_source() → Part::Image { Base64("...") }
  → OpenAiClient encodes to API format
```

### Multimodal Processing Paths

1. **Native** — agent runs on a vision model, multimodal Parts flow directly in messages
2. **Specialist** — delegate to a domain-expert agent (doc-shard, vision-analyzer)
3. **Tool fallback** — `multimodal_analyze` tool makes a one-shot call to the default vision model from settings
4. **No capability** — clear error, user configures a vision model

### `multimodal_analyze` Tool

Universal vision fallback available to all agents (root + subagents). Makes a direct HTTP call to the configured multimodal provider. Any agent on any model can process visual content.

```
Agent calls multimodal_analyze({ content: [{ type: "image", source: "/path/to/img.png" }], prompt: "..." })
  → Tool reads multimodal_config from executor state (baseUrl, apiKey, model)
  → Resolves file to base64, builds OpenAI content array
  → POST {baseUrl}/chat/completions
  → Returns { "analysis": "..." }
```

Configured via Settings > Advanced > Multimodal (provider + model with vision capability).

### Implementation Files

| File | Purpose |
|------|---------|
| `runtime/agent-primitives/src/types.rs` | Part enum (Text, Image, File), ContentSource, ImageDetail |
| `runtime/agent-primitives/src/multimodal.rs` | flush_part_to_disk, rehydrate_source, MIME utils |
| `runtime/agent-runtime/src/types/messages.rs` | ChatMessage with Vec<Part> content, custom serde |
| `runtime/agent-runtime/src/llm/openai.rs` | OpenAiClient with FileRef rehydration |
| `runtime/agent-tools/src/tools/multimodal.rs` | multimodal_analyze tool |
| `gateway/gateway-services/src/settings.rs` | MultimodalConfig in ExecutionSettings |
| `gateway/gateway-execution/src/invoke/executor.rs` | Injects multimodal_config into executor state |

## First-Time Setup Wizard

A 6-step onboarding wizard at `/setup` that configures a fresh z-Bot installation.

### Trigger

On app load, `SetupGuard` calls `GET /api/setup/status`. If `setupComplete === false` AND no providers exist, redirects to `/setup`. Result cached in sessionStorage.

### Steps

| Step | Name | Required | Persisted When |
|------|------|----------|----------------|
| 1 | Name Your Agent | Yes | On Launch (Step 6) |
| 2 | Connect Providers | Yes (≥1 verified) | Immediately (test & add) |
| 3 | Enable Skills | No (skippable) | Informational only |
| 4 | MCP Servers | No (skippable) | On Launch (Step 6) |
| 5 | Configure Agents | Yes | On Launch (Step 6) |
| 6 | Review & Launch | — | Submits all deltas |

### Delta-Only Updates

On re-run, the wizard hydrates from current state (providers, agent configs, MCPs, agent name). Launch only applies changes:
- Root agent renamed only if name changed
- Agent configs updated only where provider/model/temp/tokens differ from original
- MCP servers created only if not already existing
- `setupComplete` set to `true`, `agentName` persisted (also updates SOUL.md)

### Bundled Templates

| Template | Contents |
|----------|----------|
| `gateway/templates/default_agents.json` | 7 agents: code, data-analyst, planner, research, summarizer, tutor, writing (with temps, maxTokens, skill/MCP refs) |
| `gateway/templates/default_mcps.json` | 8 MCP servers: time, github, brave-search, google-maps, sequential-thinking, google-drive, drawio, drawio-sse (all keys blanked, disabled) |

### Name Presets

Step 1 offers quick-pick personalities: **Brahmi**, **JohnnyLever**, **z-Bot**, or custom. The chosen name is stored in `settings.json` (`execution.agentName`) and written to `config/SOUL.md` (first line: `You are **Name**`).

### Implementation Files

| File | Purpose |
|------|---------|
| `gateway/gateway-services/src/settings.rs` | `ExecutionSettings` with `setup_complete`, `agent_name` |
| `gateway/src/http/setup.rs` | Setup status + MCP defaults endpoints |
| `gateway/src/http/settings.rs` | Updates SOUL.md when `agentName` changes |
| `gateway/templates/default_agents.json` | Bundled agent definitions |
| `gateway/templates/default_mcps.json` | Sanitized MCP server templates |
| `apps/ui/src/features/setup/SetupWizard.tsx` | Wizard container, `useReducer` state, hydration |
| `apps/ui/src/features/setup/SetupGuard.tsx` | Route guard — checks status, redirects |
| `apps/ui/src/features/setup/steps/` | 6 step components (Name, Providers, Skills, Mcp, Agents, Review) |
| `apps/ui/src/features/setup/components/` | StepIndicator, WizardNav |
| `apps/ui/src/features/setup/presets.ts` | Name preset data |

### UI Architecture

Wizard renders outside the app shell (no sidebar). State managed via `useReducer` with `HYDRATE` action for pre-filling on re-run. Each step is a standalone component receiving wizard state + onChange callback. CSS uses BEM classes under `.setup-wizard`, `.step-indicator`, `.name-preset`, `.provider-add-*`, `.skill-category`, `.mcp-*`, `.agent-*`, `.review-*`.

## Memory Brain

The memory layer is z-Bot's cognitive system. Full documentation: [components/memory-layer/overview.md](components/memory-layer/overview.md). Backlog: [components/memory-layer/backlog.md](components/memory-layer/backlog.md).

### Memory Loops

| Loop | When | What | Files |
|------|------|------|-------|
| System recall | First message | `recall_with_graph()` → facts + episodes + graph → system message | `runner.rs` |
| Intent + memory | Before intent LLM | `recall_for_intent()` → corrections, strategies, episodes | `intent_analysis.rs` |
| Subagent priming | Delegation spawn | `recall_for_delegation()` → corrections, skills, ward files | `spawn.rs` |
| Mid-session | Every N turns | `RecallHook` → new relevant facts injected | `executor.rs` |
| Distillation | Session end | LLM extracts facts (verified), entities (normalized), relationships (deduped), episodes | `distillation.rs` |
| Sleep-time pipeline | Background / on tick | Synthesis, belief synthesis, corrections abstraction, conflict resolution, pairwise entity verification, contradiction detection, handoff summarization | `gateway-memory/src/sleep/*.rs` |

### Cognitive Layers

The plain `memory_facts` table is the floor — additional layers add structure, time, and abstraction:

| Layer | What it adds | Storage |
|-------|--------------|---------|
| Knowledge graph | Typed entities + relationships, 2-hop CTE expansion | `knowledge.db` |
| Episodic memory | Goal / outcome / tools-used per session | `session_episodes` |
| Bi-temporal facts | `valid_from` / `valid_to` columns; point-in-time recall, supersession writer | `memory_facts` columns |
| Hierarchical memory (HiRAG / LeanRAG) | Cluster summaries; recall walks down from LCA cluster | `memory_hierarchy_*` tables |
| Belief network | Multi-fact beliefs with confidence propagation + contradiction graph | `beliefs`, `belief_*` tables |
| Procedures | Learned, named runbooks; dispatchable via `run_procedure` tool | `procedures` table |
| Corrections abstractor | Recurring fixes promoted to high-priority rules | `memory_facts` (category=correction) |

Hierarchy + LCA recall is feature-flagged via `execution.memory.hierarchy.enabled`.

### Subagent Tool Registry

All subagents (planner, code-agent, research-agent, etc.) carry:

| Tool | Purpose |
|------|---------|
| ShellTool | Run commands, read files |
| WriteFileTool | Create/overwrite files |
| EditFileTool | Find-and-replace edits |
| LoadSkillTool | Load skill instructions |
| **WardTool** | Enter ward → doctrine + ward-entry recall |
| **MemoryTool** | recall / save_fact / graph |
| **RunProcedureTool** | Replay a learned procedure |
| **GrepTool** | Search files efficiently |
| **WaitAgentTool / KillAgentTool** | Block on / cancel a running subagent (orchestrator only) |
| RespondTool | Return result to parent |

### Recall Scoring

```
score = (0.7 × vector + 0.3 × BM25)
        × category_weight × ward_affinity × temporal_decay
        × mention_boost × contradiction_penalty × predictive_boost
        × entity_confidence (graph-ANN path)
```

FTS5 queries sanitized with OR-joined terms (raw user messages break FTS5 syntax). When hierarchy is enabled, recall is LCA-bounded — it walks the cluster tree down from the lowest-common-ancestor of seed entities and surfaces inter-cluster relations as a separate result band.

### Accuracy Layer

- **Fact verification**: grounded against tool outputs (confidence scaled by match ratio)
- **Fact dedup**: 60% word overlap check in distillation prevents near-duplicates
- **Entity normalization**: file basename matching, alias tracking in properties
- **Relationship dedup**: unique index on (source_entity_id, target_entity_id, relationship_type)
- **Failed episode warnings**: surface in recall as "Warnings — avoid these approaches" before successes
- **Pairwise verifier (sleep-time)**: LLM adjudicates proposed entity merges; defaults to deny on any LLM failure
- **Confidence-aware traversal**: graph-ANN recall weighs by entity confidence; contradicted facts are penalized

## Wards — Domain-Scoped Delegatable Agents

Wards are simultaneously **persistent working directories** and **delegatable specialist agents**. Each ward has its own doctrine (`AGENTS.md` — scope, mandate, runtime hints all in one file), optional per-ward LLM config (`config.yaml`), curated memory (`memory-bank/`), and project files. There is no separate `ZBOT.md`; the ward agent's system prompt is assembled from the global system-context shards plus the ward's `AGENTS.md` (see `gateway/gateway-execution/src/invoke/setup.rs::synthesize_ward_agent`).

### Lifecycle

| Path | Trigger | What happens |
|------|---------|--------------|
| **Warm** | Intent analysis matches an existing ward | Orchestrator delegates directly |
| **Cold** | No matching ward | Planner constructs doctrine + procedures, then runs |
| **Graduation gate** | Ward telemetry threshold met (success rate, fact density) | Ward becomes routable from warm path |
| **Out-of-scope re-route** | Request lands in wrong ward | Runtime re-delegates rather than letting mandate stretch |
| **Anti-fragmentation** | Naming collision detected | Reject `maritime-tracking` / `maritime-vessel-tracking` style splits |

### Ward Curator (autonomous self-improvement)

| Phase | Trigger | What it does |
|-------|---------|--------------|
| **A — Telemetry** | Always-on, per session | Records success/failure, fact growth, recall hits per ward |
| **B — Heuristic cleanup** | Weekly cron | Consolidates stale facts, prunes archive, dedupes |
| **C — LLM consolidation** | Periodic | LLM pass merges redundant rules, sharpens doctrine, surfaces conflicts (routed via `execution.curator`) |

The Curator's LLM model is independently configurable via the per-task routing chain — see **Per-Task LLM Routing** above.

### Per-Ward LLM Config

`wards/{name}/config.yaml`:

```yaml
provider: null   # null = inherit from orchestrator
model: null      # null = provider default
```

A `non-null` value overrides the orchestrator for that ward only. This is the per-ward override; it does **not** flow through the `LlmClientConfig.task` chain.

## Crate Structure

### Layer Overview

```
zbot/
├── runtime/        # Shared primitives, Rig-backed execution engine, built-in tools
├── stores/         # Persistence traits, domain types, SQLite backend, conformance
├── services/       # Standalone data services
├── gateway/        # HTTP/WebSocket server
├── apps/           # Applications (daemon, cli, ui)
└── dist/           # Frontend build output
```

### Runtime (`runtime/`)

Shared primitives, Rig adapter, execution engine, and built-in tools:

```
runtime/
├── agent-primitives/    # Tool/context/event/content/filesystem primitives
├── agent-runtime/       # Rig adapter, executor facade, LLM client, middleware
└── agent-tools/         # Built-in tool implementations
```

### Stores (`stores/`)

Persistence traits, domain types, SQLite implementation, and conformance:

```
stores/
├── zbot-stores-domain/       # Serde-only domain/request types
├── zbot-stores-traits/       # Dependency-light store traits
├── zbot-stores/              # Persistence facade and trait re-exports
├── zbot-stores-sqlite/       # SQLite backend, schema, repositories
└── zbot-stores-conformance/  # Backend conformance harness
```

### Services (`services/`)

Standalone data services:

```
services/
├── execution-state/     # Session/execution state machine (SQLite)
├── api-logs/            # Execution logging (SQLite)
├── knowledge-graph/     # Entity/relationship storage, GraphTraversal trait (SQLite CTE → Neo4j swappable)
└── daily-sessions/      # Session management
```

### Gateway (`gateway/`)

Network layer, decomposed into focused crates:

```
gateway/
├── gateway-events/      # EventBus, GatewayEvent, HookContext
├── gateway-templates/   # Prompt assembly, shard injection
├── gateway-connectors/  # ConnectorRegistry, dispatch (Discord, Telegram, Slack)
├── gateway-services/    # AgentService, ProviderService, ModelRegistry, McpService, SkillService, SettingsService
├── gateway-execution/   # ExecutionRunner, delegation, lifecycle, streaming, BatchWriter, SessionDistiller (health, episodes, strategies, failure clustering, ward sync), MemoryRecall (priority engine, graph expansion, nudges)
├── gateway-hooks/       # Hook trait, HookRegistry, CliHook, CronHook
├── gateway-cron/        # CronJobConfig, CronService
├── gateway-bus/         # GatewayBus trait, SessionRequest, SessionHandle
├── gateway-ws-protocol/ # ClientMessage, ServerMessage, SubscriptionScope
├── src/                 # Thin shell: HTTP routes, WebSocket handler, AppState
└── templates/           # System prompt templates (embedded at compile time)
```

### Apps (`apps/`)

Runnable applications:

```
apps/
├── cli/                 # CLI tool with TUI (binary: zbot)
├── daemon/              # Main binary (zbotd)
└── ui/                  # React/Vite web dashboard
```

## Core Abstractions

### Agent Engine Facade
```rust
#[async_trait]
pub trait AgentEngine: Send {
    fn engine_name(&self) -> &'static str;

    async fn execute_stream(
        &mut self,
        user_message: &str,
        history: &[ChatMessage],
        on_event: StreamCallback,
    ) -> Result<(), ExecutorError>;
}
```

### Tool Trait
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Option<Value>;
    fn permissions(&self) -> ToolPermissions;

    async fn execute(
        &self,
        ctx: Arc<dyn ToolContext>,
        args: Value,
    ) -> Result<Value>;
}
```

### LLM Client
```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat_completion_stream(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Value]>,
        callback: &mut dyn FnMut(StreamEvent),
    ) -> Result<()>;
}
```

### LLM Client Wrapping Chain

The LLM client is wrapped in a chain of decorators for reliability and rate limiting:

```
OpenAiClient → RetryingLlmClient → RateLimitedLlmClient
```

| Layer | Purpose | File |
|-------|---------|------|
| `OpenAiClient` | Raw OpenAI-compatible API calls | `runtime/agent-runtime/src/llm/openai.rs` |
| `RetryingLlmClient` | Automatic retry on transient errors | `runtime/agent-runtime/src/llm/retry.rs` |
| `RateLimitedLlmClient` | Per-provider concurrency semaphore + sliding window RPM | `runtime/agent-runtime/src/llm/rate_limiter.rs` |

The `RateLimitedLlmClient` uses a shared `ProviderRateLimiter` per provider (concurrency semaphore + RPM sliding window). All executors for the same provider share the same limiter, preventing burst 429s. Auto-halves RPM after a 429. Configured via `rateLimits` in `providers.json` (default: 30 RPM / 2 concurrent).

## Session Management Architecture

Sessions are the top-level container for user interactions. Each session has one continuous
message stream — all tool calls, results, and intermediate context persist across user messages.
Subagents get isolated context via child sessions.

### Session Tree

```
ROOT SESSION (parent_session_id = NULL)
│
├── messages stream (ALL messages — continuous across user turns)
│   ├── user: "build me a docx"
│   ├── assistant: [tool_calls: list_skills]
│   ├── tool: "16 skills available..."              (tool_call_id: call_001)
│   ├── assistant: [tool_calls: shell(pip install)]
│   ├── tool: "installed python-docx"               (tool_call_id: call_002)
│   ├── assistant: "Done! Created the docx file."
│   ├── user: "convert to pdf"                       ← 2nd message, SAME session
│   ├── assistant: [tool_calls: shell(libreoffice)]
│   ├── tool: "converted to /tmp/out.pdf"            (tool_call_id: call_003)
│   ├── assistant: "Done! PDF ready."
│   └── system: "## From Researcher\n..."            ← callback from child
│
├── exec-{uuid} (root, REUSED across all user messages)
│
└── CHILD SESSION (parent_session_id = root session)
    ├── messages stream (ISOLATED — only subagent sees these)
    │   ├── user: "research X for the docx"
    │   ├── assistant: [tool_calls: web_fetch]
    │   ├── tool: "fetched data..."
    │   └── assistant: "Found Y. Here's the summary."
    └── exec-{uuid} (root of child session)
```

### Session Lifecycle

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         SESSION LIFECYCLE                                │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   User sends first message (no session_id)                              │
│        │                                                                │
│        ▼                                                                │
│   ┌─────────────────┐                                                   │
│   │ Create Session  │ ──► sess-{uuid} created in DB                     │
│   │ (status=running)│     source = web|cli|api|cron|connector           │
│   └────────┬────────┘                                                   │
│            │                                                            │
│            ▼                                                            │
│   ┌─────────────────┐                                                   │
│   │ Create Root     │ ──► exec-{uuid} created, delegation_type=root     │
│   │ Execution       │                                                   │
│   └────────┬────────┘                                                   │
│            │                                                            │
│            ▼                                                            │
│   ┌─────────────────┐                                                   │
│   │ Stream messages │ ──► user, assistant, tool messages appended        │
│   │ to session      │     to session stream as they happen              │
│   └────────┬────────┘                                                   │
│            │                                                            │
│            ▼                                                            │
│   User sends follow-up message (WITH session_id)                        │
│        │                                                                │
│        ▼                                                                │
│   ┌─────────────────┐                                                   │
│   │ Reuse root      │ ──► Same session, same root execution             │
│   │ execution       │     Reactivated if completed/crashed              │
│   └────────┬────────┘     Full conversation history available           │
│            │                                                            │
│            ▼                                                            │
│   Delegation spawns child session                                       │
│        │                                                                │
│        ▼                                                                │
│   ┌─────────────────┐                                                   │
│   │ Child session   │ ──► sess-{uuid} with parent_session_id set        │
│   │ (isolated)      │     Subagent messages go to child stream          │
│   └────────┬────────┘     Callback result posted to parent stream       │
│            │                                                            │
│            ▼                                                            │
│   User sends /new command                                               │
│        │                                                                │
│        ▼                                                                │
│   ┌─────────────────┐                                                   │
│   │ Clear session_id│ ──► localStorage cleared                          │
│   │ from frontend   │     Next message creates new session              │
│   └─────────────────┘                                                   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Delegation

When the root agent delegates to a subagent, the following constraints apply:

| Aspect | Behavior |
|--------|----------|
| **Concurrency limit** | Max 3 concurrent delegations via `tokio::sync::Semaphore` |
| **Child session lifecycle** | Child sessions are marked `completed` when subagent finishes (no orphans) |
| **Subagent context** | Subagents receive `fact_store` with embeddings (no file fallback) |
| **LLM throttle** | Subagents share the provider's `ThrottledLlmClient` semaphore |
| **Intent analysis** | Subagents skip intent analysis (root-agent only) |

### Session vs Execution vs Conversation

| Concept | Scope | Purpose |
|---------|-------|---------|
| **Session** (`sess-{uuid}`) | User work session | Groups all messages until `/new`. One continuous stream. |
| **Execution** (`exec-{uuid}`) | Agent lifetime | Root execution reused across messages. Child executions for subagents. |
| **Conversation ID** (`web-{uuid}`) | Client-side only | Generated in localStorage for WebSocket event routing. NOT in core DB schema. |

### Session and Execution States

**Session Status:**
| Status | Description |
|--------|-------------|
| `queued` | Created but not yet started |
| `running` | Actively processing |
| `paused` | Paused by user or server shutdown |
| `completed` | Successfully finished |
| `crashed` | Failed with error or unexpected interruption |

**Execution Status:**
| Status | Description |
|--------|-------------|
| `queued` | Created but not yet started |
| `running` | Actively executing |
| `paused` | Paused (session paused or waiting) |
| `completed` | Successfully finished |
| `crashed` | Failed with error |
| `cancelled` | Cancelled by user or parent |

### Server Shutdown Behavior

The server handles session states differently based on shutdown type:

**Graceful Shutdown (Ctrl+C):**
- All running sessions are marked as `paused`
- All running/queued executions are marked as `paused`
- Sessions can be resumed when the server restarts

**Unexpected Crash:**
- Sessions remain in `running` state in the database
- On startup, any sessions still in `running` state are marked as `crashed`
- This indicates they were interrupted unexpectedly

```
Graceful Shutdown:
  Server receives SIGINT/SIGTERM
       │
       ▼
  mark_running_as_paused()  ──► Sessions: running → paused
       │                        Executions: running/queued → paused
       ▼
  Shutdown HTTP/WebSocket servers

Startup Recovery:
  Server starts
       │
       ▼
  mark_running_as_crashed()  ──► Only sessions still in "running" state
       │                         (unexpected crash) marked as crashed
       ▼
  Normal operation
```

### Frontend Session Persistence

The frontend stores session state in localStorage:

```typescript
// Keys used for session persistence
const WEB_SESSION_ID_KEY = 'zbot_web_session_id';
const WEB_CONV_ID_KEY = 'zbot_web_conv_id';

// On agent_started event, store session_id
localStorage.setItem(WEB_SESSION_ID_KEY, event.session_id);

// On subsequent messages, include session_id
{ type: "invoke", session_id: storedSessionId, ... }

// On /new command, clear session
localStorage.removeItem(WEB_SESSION_ID_KEY);
```

### Trigger Sources

Sessions track their origin for analytics and UI filtering:

| Source | Value | Auto-complete | Description |
|--------|-------|---------------|-------------|
| Web | `web` | No | Interactive web UI sessions (stays open for follow-up) |
| CLI | `cli` | Yes | Command line invocations |
| Cron | `cron` | Yes | Scheduled job triggers |
| API | `api` | Yes | Direct `POST /api/gateway/submit` calls |
| Connector | `connector` | Yes | External worker inbound messages (also accepts `plugin` alias) |

**Auto-complete**: Sessions from CLI, Cron, API, and Connector sources automatically complete after execution finishes. Web sessions stay open for interactive multi-turn use.

### Invocation Methods

| Method | Endpoint/Message | Source |
|--------|------------------|--------|
| Web chat | WebSocket `invoke` | Defaults to `web` |
| Connector inbound (HTTP) | `POST /api/connectors/:id/inbound` | Server sets `connector` |
| Connector inbound (WebSocket) | Worker `inbound` message | Server sets `connector` |
| Gateway submit | `POST /api/gateway/submit` | Caller specifies in payload |
| Cron trigger | Internal scheduler | Server sets `cron` |

#### POST /api/gateway/submit

For direct API access, include `source` in the request body:

```json
{
  "agent_id": "root",
  "message": "Hello",
  "source": "api",
  "conversation_id": "optional-conv-id",
  "session_id": "optional-existing-session"
}
```

The `source` field is optional and defaults to `web`. Valid values: `web`, `cli`, `cron`, `api`, `connector`.

## Execution Flow

```
User Message
     │
     ▼
┌─────────────────┐
│   WebSocket     │ ◄── { type: "invoke", session_id?, message: "..." }
│   Handler       │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Session       │
│   Resolution    │
├─────────────────┤
│ if session_id { │
│   lookup(id)    │ ──► Reuse session + root execution
│   reactivate()  │     (reactivate if completed/crashed)
│ } else {        │
│   create_new()  │ ──► New session + root execution
│ }               │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Execution     │
│   Runner        │
├─────────────────┤
│ 1. Load agent   │
│ 2. Load history │ ◄── get_session_conversation(session_id, 200)
│ 3. Create LLM   │     Full conversation with tool calls
│ 4. Build tools  │
└────────┬────────┘
         │
         ▼
┌──────────────────────────────────────────────────────────┐
│   Agent Executor (messages streamed via BatchWriter)     │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  append_message(user, input)        ──► session stream   │
│                                                          │
│  while !done {                                           │
│    llm.call()                       ──► tokens → WS → UI│
│    append_message(assistant, text+tool_calls)             │
│    if tool_call {                                        │
│      execute()                      ──► result → WS → UI│
│      append_message(tool, result, tool_call_id)          │
│    }                                                     │
│  }                                                       │
│                                                          │
│  append_message(assistant, final_response)               │
│                                                          │
└──────────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────┐
│  Update Session │ ──► Status, token aggregation
└─────────────────┘
```

## API Reference

### HTTP Endpoints (port 18791)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/health` | Health check |
| GET | `/api/status` | Daemon status |
| GET | `/api/agents` | List agents |
| POST | `/api/agents` | Create agent |
| GET | `/api/agents/:id` | Get agent |
| PUT | `/api/agents/:id` | Update agent |
| DELETE | `/api/agents/:id` | Delete agent |
| GET | `/api/providers` | List providers |
| POST | `/api/providers` | Create provider |
| POST | `/api/providers/:id/default` | Set default |
| POST | `/api/providers/test` | Test connection |
| GET | `/api/skills` | List skills |
| POST | `/api/skills` | Create skill |
| GET | `/api/logs/sessions` | List execution sessions |
| GET | `/api/logs/sessions/:id` | Get session with logs |
| DELETE | `/api/logs/sessions/:id` | Delete session |
| **Settings** | | |
| GET | `/api/settings/tools` | Get tool settings |
| PUT | `/api/settings/tools` | Update tool settings |
| GET | `/api/settings/logs` | Get log settings |
| PUT | `/api/settings/logs` | Update log settings (requires restart) |
| **Execution Intelligence Dashboard** | | |
| GET | `/api/executions/stats/counts` | KPI cards (success rate, tokens, tool calls, duration) |
| GET | `/api/executions/v2/sessions/full` | Sessions with inline mini waterfalls |
| GET | `/api/executions/v2/sessions/:id` | Full waterfall timeline with delegation spans |
| **Observatory (Knowledge Graph)** | | |
| GET | `/api/memory/graph/entities` | Graph entities for D3-force visualization |
| GET | `/api/memory/graph/relationships` | Graph relationships |
| GET | `/api/memory/health` | Learning health (distillation stats) |
| POST | `/api/memory/distill/backfill` | Retroactive distillation |
| POST | `/api/gateway/submit` | Submit new agent request |
| GET | `/api/gateway/status/:session_id` | Get session status |
| POST | `/api/gateway/cancel/:session_id` | Cancel running session |

### WebSocket Protocol (`ws://host:18791/ws`)

Served on the HTTP port via the `/ws` upgrade route — no second firewall hole. A legacy standalone WebSocket bind on port 18790 still exists for external integrations that hardcoded `ws://host:18790`; it is **off by default** and lit by setting `legacy_ws_port_enabled: true` in `gateway/src/config.rs`. Slated for removal.

**Client Commands:**
```typescript
// Invoke agent (session_id optional - if omitted, new session created)
{
  type: "invoke",
  agent_id: string,
  conversation_id: string,
  message: string,
  session_id?: string  // Include to continue existing session
}

// Stop execution
{ type: "stop", conversation_id: string }

// Continue after max iterations
{ type: "continue", conversation_id: string }

// Subscribe to events with scope filtering
{
  type: "subscribe",
  conversation_id: string,  // Session ID to subscribe to
  scope: "all" | "session" | "execution:{exec_id}"
}
// Scopes:
// - "all": All events (backward compatible, includes subagent internal events)
// - "session": Root execution events + delegation lifecycle markers only
// - "execution:{id}": All events for a specific execution (debug view)

// Unsubscribe
{ type: "unsubscribe", conversation_id: string }
```

**Subscription Response:**
```typescript
// Subscription confirmed
{
  type: "subscribed",
  conversation_id: string,
  current_sequence: number,
  root_execution_ids?: string[]  // For session scope, list of root execution IDs
}
```

**Server Events:**
```typescript
// Agent started processing (IMPORTANT: contains session_id for client to store)
{
  type: "agent_started",
  agent_id: string,
  conversation_id: string,
  session_id: string,      // Client should store this for subsequent messages
  execution_id: string     // Unique execution within session
}

// Streaming token
{ type: "token", agent_id: string, conversation_id: string, delta: string }

// Tool being called
{ type: "tool_call", agent_id: string, conversation_id: string,
  tool_id: string, tool_name: string, args: object }

// Tool result
{ type: "tool_result", agent_id: string, conversation_id: string,
  tool_id: string, result: string, error?: string }

// Agent finished
{ type: "agent_completed", agent_id: string, conversation_id: string,
  result: string }

// Error occurred
{ type: "error", agent_id?: string, conversation_id?: string,
  message: string }
```

## Database Schema

### Entity Relationships

```
sessions ||--o{ sessions : "parent-child (delegation)"
sessions ||--o{ agent_executions : contains
sessions ||--o{ messages : "conversation stream"
agent_executions ||--o{ agent_executions : "parent-child (delegation)"
```

### sessions
Top-level container. Root sessions have `parent_session_id = NULL`.
Child sessions (for subagents) link back to their parent.

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,                    -- sess-{uuid}
    status TEXT NOT NULL,                   -- queued|running|completed|crashed|cancelled
    source TEXT NOT NULL,                   -- web|cli|api|cron|connector
    root_agent_id TEXT NOT NULL,
    title TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    error_message TEXT,                     -- null unless crashed
    total_tokens_in INTEGER DEFAULT 0,
    total_tokens_out INTEGER DEFAULT 0,
    metadata TEXT,                          -- JSON
    pending_delegations INTEGER DEFAULT 0,  -- Count of running subagents
    continuation_needed INTEGER DEFAULT 0,  -- Flag for continuation after delegates
    ward_id TEXT,                           -- Active code ward name
    parent_session_id TEXT                  -- NULL=root, sess-{uuid}=child (subagent)
);
```

### agent_executions
An agent's participation in a session. Root execution is reused across user messages.

```sql
CREATE TABLE agent_executions (
    id TEXT PRIMARY KEY,                    -- exec-{uuid}
    session_id TEXT NOT NULL REFERENCES sessions(id),
    agent_id TEXT NOT NULL,
    parent_execution_id TEXT REFERENCES agent_executions(id),
    delegation_type TEXT NOT NULL,          -- root|sequential|parallel
    task TEXT,                              -- Task description for delegated agents
    status TEXT NOT NULL,                   -- queued|running|paused|completed|crashed|cancelled
    started_at TEXT,
    completed_at TEXT,
    tokens_in INTEGER DEFAULT 0,
    tokens_out INTEGER DEFAULT 0,
    checkpoint TEXT,                        -- JSON for resumption
    error TEXT,
    log_path TEXT                           -- Relative path to log file
);
```

### messages
Conversation stream linked directly to sessions (not via execution JOIN).
Messages are streamed in real-time via BatchWriter as they happen.

```sql
CREATE TABLE messages (
    id TEXT PRIMARY KEY,                    -- msg-{uuid}
    execution_id TEXT,                      -- exec-{uuid}, nullable (audit trail)
    session_id TEXT,                        -- sess-{uuid}, primary FK for queries
    role TEXT NOT NULL,                     -- user|assistant|tool|system
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    token_count INTEGER DEFAULT 0,
    tool_calls TEXT,                        -- JSON array (on assistant messages)
    tool_results TEXT,                      -- JSON (legacy, unused in new path)
    tool_call_id TEXT                       -- Links tool results to their tool call
);
```

### memory_facts
Structured facts extracted from sessions (distillation) or saved manually by the agent.
Deduplication via UNIQUE(agent_id, scope, key) — repeated saves update content and bump mention_count.

```sql
CREATE TABLE memory_facts (
    id TEXT PRIMARY KEY,                         -- fact-{uuid}
    session_id TEXT,                              -- which session produced this (NULL if manual)
    agent_id TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'agent',          -- shared / agent / ward
    category TEXT NOT NULL,                       -- preference, decision, pattern, entity, instruction, correction
    key TEXT NOT NULL,                            -- dedup key: "user.preferred_language"
    content TEXT NOT NULL,                        -- 1-2 sentence fact
    confidence REAL NOT NULL DEFAULT 0.8,         -- 0.0-1.0
    mention_count INTEGER NOT NULL DEFAULT 1,
    source_summary TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT,                              -- optional TTL
    UNIQUE(agent_id, scope, key)
);
```

FTS5 virtual table `memory_facts_fts` auto-synced via INSERT/UPDATE/DELETE triggers.

### embedding_cache
Hash-based dedup for embeddings. Prevents re-embedding unchanged content.

```sql
CREATE TABLE embedding_cache (
    content_hash TEXT NOT NULL,                   -- SHA-256 of text
    model TEXT NOT NULL,                          -- which model produced this
    embedding BLOB NOT NULL,                      -- raw f32 bytes
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (content_hash, model)
);
```

### Cognitive Memory Architecture

The memory system is a full cognitive pipeline: distill (post-session extraction), recall (tool-call based retrieval with priority scoring), a knowledge graph with graph-driven expansion, and a background sleep-time pipeline that abstracts beliefs and resolves contradictions.

```
                    ┌─────────────────────────────────┐
                    │       Embedding Provider         │
                    │  (local fastembed / OpenAI /     │
                    │   Ollama / any compatible API)   │
                    └──────────┬───────────────────────┘
                               │ vectors
   ┌───────────────────────────┼──────────────────────────────┐
   ▼                           ▼                              ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────────────┐
│ Session Distiller│ │  Memory Indexer  │ │  Memory Recall            │
│ (post-session)   │ │ (on fact write)  │ │ (tool-call based)         │
│ + health report  │ │ + vec0 index     │ │ + graph expansion         │
│ + episode extract│ │                  │ │ + LCA-bounded hierarchy   │
│ + strategy emerge│ │                  │ │ + priority engine         │
│ + failure cluster│ │                  │ │ + confidence weighting    │
│ + ward file sync │ │                  │ │ + point-in-time (valid_*)│
└────────┬─────────┘ └────────┬─────────┘ └─────────────┬─────────────┘
         │                    │                          │
         ▼                    ▼                          ▼
┌──────────────────────────────────────────────────────────────────────┐
│                       conversations.db                                │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────────┐  │
│  │ memory_facts │  │ memory_facts │  │ vec0 ANN indexes           │  │
│  │ (+ valid_from│  │ _fts (FTS5)  │  │ (entity names, fact texts) │  │
│  │   valid_to)  │  │              │  │                            │  │
│  └─────────────┘  └──────────────┘  └────────────────────────────┘  │
│                                                                      │
│  ┌──────────────────┐ ┌─────────────────┐ ┌─────────────────────┐  │
│  │ session_episodes │ │ memory_hierarchy│ │ procedures           │  │
│  │ (episodic mem)   │ │ (HiRAG/LeanRAG) │ │ (replayable runbooks)│  │
│  └──────────────────┘ └─────────────────┘ └─────────────────────┘  │
│                                                                      │
│  ┌──────────────────┐ ┌─────────────────┐ ┌─────────────────────┐  │
│  │ beliefs +        │ │ recall_log      │ │ memory_facts_archive│  │
│  │ belief_contradict│ │ (audit trail)   │ │ (decay output)      │  │
│  └──────────────────┘ └─────────────────┘ └─────────────────────┘  │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
┌──────────────────────────────────────────────────────────────────────┐
│ Knowledge Graph (services/knowledge-graph/ → knowledge.db)            │
│ Entities + typed relationships, cross-agent __global__ namespace      │
│ GraphTraversal trait (SQLite CTE today, Neo4j swappable)              │
│ 2-hop BFS expansion via recursive CTE for recall                      │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▲    background tick     ┌────────────────────────────────────┐
         └────────────────────────│   Sleep-Time Pipeline               │
                                   │   (routed via execution.sleepTime) │
                                   │ • Synthesizer (facts → beliefs)    │
                                   │ • BeliefSynthesizer                │
                                   │ • CorrectionsAbstractor            │
                                   │ • PatternExtractor                 │
                                   │ • ConflictResolver                 │
                                   │ • PairwiseVerifier (entity merges) │
                                   │ • BeliefContradictionDetector      │
                                   │ • LlmAggregateEntity               │
                                   │ • HandoffWriter (conv. summary)    │
                                   └────────────────────────────────────┘

Hybrid search:    0.7 * vector + 0.3 * bm25
Recall priority:  category_weight × ward_affinity × temporal_decay
                  × mention_boost × contradiction_penalty × predictive_boost
                  × entity_confidence (graph-ANN path)
Category weights: correction 1.5× > strategy 1.4× > user 1.3× > domain 1.0×
```

#### Distillation Pipeline

Post-session LLM extraction with:
- **Health reporting**: `distillation_runs` table tracks success/failure per session
- **Provider fallback**: tries configured provider, falls back gracefully
- **Episode extraction**: identifies session episodes (goal, outcome, tools used) in `session_episodes`
- **Strategy emergence**: detects repeated successful patterns, promotes to strategy facts
- **Failure clustering**: groups repeated failures, auto-generates correction facts
- **Ward file sync**: auto-generates `wards/{ward}/memory/ward.md` from distilled knowledge
- **Contradiction detection**: flags conflicting facts via `memory_facts.contradicted_by`

#### Recall Architecture

Recall is **tool-call based** — the agent explicitly calls `memory recall` (not hidden injection). This makes recall visible, debuggable, and learnable.

**Priority scoring**: Each recalled fact is scored by:
1. **Category weight**: correction (1.5x) > strategy (1.4x) > user preference (1.3x) > domain (1.0x)
2. **Ward affinity boost**: facts from the active ward score higher
3. **Temporal decay**: per-category half-lives (corrections 90d, domain 30d) via `recall_config.json`
4. **Contradiction penalty**: facts flagged by `contradicted_by` are penalized
5. **Predictive recall**: success-correlated facts bubble up from historical recall_log

**Graph-driven expansion**: After initial fact retrieval, a 2-hop BFS via SQLite recursive CTE expands through the knowledge graph. Related entities within `max_hops` (configurable) are included with `hop_decay` attenuation.

**Corrections as rules**: Top correction facts are always injected first, formatted as "NEVER do X" / "ALWAYS do Y" rules. Filtered by query relevance.

**Capability gap detection**: When no matching skill/agent is found, recall surfaces the gap and prompts the agent to create a plan.

**Recall nudges**: System nudges at session start, ward entry, and post-delegation prompt the agent to recall via the tool.

**Configuration**: `config/recall_config.json` with `category_weights`, `ward_affinity`, `temporal_decay` half-lives, `graph_traversal` (max_hops, hop_decay), `predictive_recall`, `session_offload`.

#### Session Offload

Old session transcripts are archived to JSONL.gz files to keep SQLite lean:
- `zbot sessions archive --older-than 7` — offload transcripts older than N days
- `zbot sessions restore <session_id>` — restore an archived session
- `sessions.archived` column tracks offload state

#### Fact Pruning

Temporal decay moves old facts past their category half-life to `memory_facts_archive`. Archived facts are excluded from recall but preserved for audit.

**Key files**:
- `runtime/agent-runtime/src/llm/embedding.rs` — EmbeddingClient trait, EmbeddingConfig
- `runtime/agent-runtime/src/llm/openai_embedding.rs` — OpenAI-compatible embedding client
- `runtime/agent-runtime/src/llm/local_embedding.rs` — fastembed local client (default)
- `stores/zbot-stores-sqlite/src/memory_repository.rs` — MemoryFact CRUD, hybrid search, embedding cache
- `gateway/gateway-execution/src/distillation.rs` — SessionDistiller (health reporting, episode extraction, strategy emergence, failure clustering, ward file sync)
- `gateway/gateway-execution/src/recall.rs` — MemoryRecall (priority engine, graph expansion, corrections as rules, nudges)
- `runtime/agent-tools/src/tools/memory.rs` — save_fact, recall, graph actions
- `config/recall_config.json` — recall tuning: weights, decay, graph traversal, predictive recall

### distillation_runs
Tracks distillation health per session (v11).

```sql
CREATE TABLE distillation_runs (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    status TEXT NOT NULL,              -- success|failed|skipped
    facts_extracted INTEGER DEFAULT 0,
    entities_extracted INTEGER DEFAULT 0,
    relationships_extracted INTEGER DEFAULT 0,
    provider TEXT,
    error_message TEXT,
    duration_ms INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### session_episodes
Episodic memory — goal/outcome pairs extracted during distillation (v11).

```sql
CREATE TABLE session_episodes (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    episode_index INTEGER NOT NULL,
    goal TEXT NOT NULL,
    outcome TEXT,
    tools_used TEXT,                    -- JSON array
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### recall_log
Audit trail for recall invocations — enables predictive recall (v13).

```sql
CREATE TABLE recall_log (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    query TEXT NOT NULL,
    facts_returned INTEGER DEFAULT 0,
    graph_hops_used INTEGER DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### memory_facts_archive
Temporally decayed facts moved here for archival (v13).

```sql
CREATE TABLE memory_facts_archive (
    -- Same schema as memory_facts
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    category TEXT NOT NULL,
    key TEXT NOT NULL,
    content TEXT NOT NULL,
    confidence REAL,
    archived_at TEXT NOT NULL DEFAULT (datetime('now')),
    original_created_at TEXT
);
```

### Additional columns (migration notes)

| Migration | Table | Column | Purpose |
|-----------|-------|--------|---------|
| v11 | `memory_facts` | `ward_id TEXT` | Ward-scoped facts |
| v12 | `memory_facts` | `contradicted_by TEXT` | Links to contradicting fact ID |
| v13 | `sessions` | `archived INTEGER DEFAULT 0` | Session offload tracking |

### ID Conventions

| Table | Prefix | Example |
|-------|--------|---------|
| sessions | `sess-` | `sess-03782b12-c041-4115-9cc7-c5fcc17775a6` |
| agent_executions | `exec-` | `exec-f11b1447-9338-405c-a7d6-06f92cb87c84` |
| messages | `msg-` | `msg-28ba79f2-b386-4a1c-8e5f-1a2b3c4d5e6f` |

### Indexes

```sql
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_created ON sessions(created_at);
CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
CREATE INDEX idx_executions_session ON agent_executions(session_id);
CREATE INDEX idx_executions_parent ON agent_executions(parent_execution_id);
CREATE INDEX idx_executions_status ON agent_executions(status);
CREATE INDEX idx_executions_agent ON agent_executions(agent_id);
CREATE INDEX idx_messages_execution ON messages(execution_id);
CREATE INDEX idx_messages_created ON messages(created_at);
CREATE INDEX idx_messages_session ON messages(session_id);
CREATE INDEX idx_messages_session_created ON messages(session_id, created_at);
```

### Status Semantics

**Session Status:**
| Status | Description |
|--------|-------------|
| `queued` | Created but not yet started |
| `running` | At least one agent execution is running |
| `completed` | All executions completed successfully |
| `crashed` | Root execution crashed |
| `cancelled` | User cancelled the session |

**Execution Status:**
| Status | Description |
|--------|-------------|
| `queued` | Waiting to start |
| `running` | Currently executing |
| `paused` | Paused (session paused or waiting) |
| `completed` | Finished successfully |
| `crashed` | Failed with error |
| `cancelled` | Cancelled by user or parent |

## Built-in Tools

### Core Tools

| Tool | Description | Permissions |
|------|-------------|-------------|
| `shell` | Primary execution — commands and scripts (file writes rejected — use write_file / edit_file) | Dangerous |
| `write_file` | Create new files inside the active ward | Dangerous |
| `edit_file` | Targeted find-and-replace edits on existing files | Dangerous |
| `memory` | Persistent KV store + save_fact + recall + graph | Safe |
| `ward` | Manage code wards (use, list, create, info) | Safe |
| `update_plan` | Lightweight task checklist | Safe |
| `set_session_title` | Set a human-readable session title | Safe |
| `execution_graph` | DAG workflow engine for multi-step orchestration | Safe |
| `list_skills` | List available skills | Safe |
| `load_skill` | Load skill instructions | Safe |
| `grep` | Search file contents | Safe |

### Action Tools (Always Enabled)

| Tool | Description | Permissions |
|------|-------------|-------------|
| `respond` | Send response to user | Safe |
| `delegate_to_agent` | Delegate task to subagent | Safe |
| `list_agents` | List available agents | Safe |

### Optional Tools (Configurable)

| Tool | Description | Permissions |
|------|-------------|-------------|
| `read` | Read file contents | Safe |
| `write` | Write content to file | Moderate |
| `edit` | Edit file contents | Moderate |
| `glob` | Find files by pattern | Safe |
| `todos` | Heavyweight task persistence (SQLite) | Safe |
| `python` | Execute Python code | Dangerous |
| `web_fetch` | Fetch web content | Moderate |
| `ui_tools` | UI manipulation tools | Moderate |
| `create_agent` | Create new agents | Moderate |
| `introspection` | Agent introspection (list_tools, list_mcps) | Safe |

## Resource Indexing System

Skills and agents are indexed for semantic search and relationship tracking. The system uses a **lazy indexing** approach — indexing happens on-demand, not at startup.

### Index Storage

| Storage | Purpose | Persistence |
|---------|---------|-------------|
| **Memory Fact Store** | Semantic search (BM25 + vector embeddings) | SQLite + FTS5 + embeddings |
| **Knowledge Graph** | Entity/relationship storage | SQLite via GraphStorage |
| **Context State Cache** | Fast lookup during session | Per-session (index:skills, index:agents) |

### Indexing Flow

```
index_resources called (or first discovery)
     │
     ▼
┌─────────────────────────────────────────┐
│ 1. Scan skills_dir/ for SKILL.md files  │
│    → Parse frontmatter                  │
│    → Build SkillMetadata                │
│                                         │
│ 2. Scan agents_dir/ for config.yaml     │
│    → Parse YAML                         │
│    → Build AgentMetadata                │
│                                         │
│ 3. Store in Memory Fact Store           │
│    → Category: "skill" or "agent"       │
│    → Key: "skill:{name}" or "agent:{name}"  │
│    → Content: name + description + keywords   │
│                                         │
│ 4. Store in Knowledge Graph             │
│    → Entity type: "skill" or "agent"    │
│    → Properties: description, tools, etc.│
│                                         │
│ 5. Cache mtimes in context state        │
│    → index:skills_mtimes                │
│    → index:agents_mtimes                │
└─────────────────────────────────────────┘
```

### Discovery Flow

Resources are discovered through two paths:

**Intent analysis middleware** (autonomous, pre-execution):
```
┌─────────────────────────────────────────────────────────────────┐
│ 1. Index all resources into memory_facts (idempotent upsert)   │
│    → Skills, agents, wards indexed with local embeddings       │
│                                                                 │
│ 2. Semantic search via recall_facts("root", message, 50)       │
│    → Filter by score ≥ 0.15                                    │
│    → Cap: 8 skills, 5 agents, 5 wards                          │
│                                                                 │
│ 3. Top-N results sent to LLM for analysis                      │
└─────────────────────────────────────────────────────────────────┘
```

**Tool-based discovery** (list_skills, list_agents):
```
┌─────────────────────────────────────────────────────────────────┐
│ 1. Try cached index from context state                          │
│    → index:skills, index:agents                                 │
│                                                                 │
│ 2. Fall back to disk scan                                       │
│    → Parse SKILL.md/config.yaml on-demand                       │
└─────────────────────────────────────────────────────────────────┘
```

### When Indexing Happens

| Trigger | Behavior |
|---------|----------|
| Intent analysis middleware | Indexes skills, agents, wards into `memory_facts` every root session (idempotent upsert) |
| `index_resources()` tool called | Full reindex (or force=true for stale) |
| File modification detected | Staleness check during next indexing |

### Error Recovery

When `load_skill` or agent loading fails:
1. File not found → Remove from index automatically
2. Corrupted file → Suggest `index_resources(force=true)`

## Intent Analysis System

Intent analysis is an **autonomous pre-execution middleware** — not a tool agents call. It indexes resources into `memory_facts` with local embeddings (fastembed), performs semantic search, sends only top-N relevant resources to a single LLM call, and injects the result as a `## Intent Analysis` section into the system prompt. See `docs/architecture/components/intent-analysis/overview.md` for full documentation.

Implementation: `gateway/gateway-execution/src/middleware/intent_analysis.rs`

### Architecture

| Aspect | Design |
|--------|--------|
| **Trigger** | Middleware, before root agent's first LLM call |
| **Scope** | Root agent only — subagents and continuations skip it |
| **Resource Discovery** | Autonomous: indexes skills/agents/wards into `memory_facts`, searches semantically |
| **LLM Input** | Top-N relevant resources only (not full catalog) |
| **Filtering** | Score threshold (0.15), per-category caps (8 skills, 5 agents, 5 wards) |
| **Side Effects** | None — injects guidance text, does not load skills or delegate |
| **Agent Visibility** | Sees `## Intent Analysis` section in system prompt from turn one |

### Flow

```
User Message
     │
     ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 1: Index resources (idempotent upsert)                 │
│   Skills → memory_facts (category:"skill")                  │
│   Agents → memory_facts (category:"agent")                  │
│   Wards  → memory_facts (category:"ward", reads AGENTS.md) │
└─────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 2: Semantic search (recall_facts with fastembed)        │
│   Fetch top 50, filter by score ≥ 0.15                      │
│   Cap: 8 skills, 5 agents, 5 wards                          │
└─────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────┐
│ Step 3: LLM call with top-N resources                       │
│   Output: IntentAnalysis { primary_intent, hidden_intents,  │
│     recommended_skills, recommended_agents,                  │
│     ward_recommendation { action, ward_name, subdirectory,  │
│                           structure, reason },               │
│     execution_strategy { approach, graph, explanation },     │
│     rewritten_prompt }                                       │
└─────────────────────────────────────────────────────────────┘
     │
     (parse failed? skip enrichment, continue with base prompt)
     ▼
┌─────────────────────────────────────────────────────────────┐
│ inject_intent_context()                                     │
│  Appends "## Intent Analysis" section to system prompt      │
└─────────────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────────────────────┐
│ Executor starts with enriched system prompt                 │
│  - No conditional dispatch code in runner                   │
│  - LLM reads the section and decides how to proceed         │
└─────────────────────────────────────────────────────────────┘
```

### Key Behavioral Contract

- Enrichment is automatic and transparent — agents do not call `analyze_intent`
- Resource discovery is autonomous — indexes into `memory_facts`, searches via embeddings
- Hidden intents are actionable instructions, not category labels
- Runner contains no conditional logic based on analysis output — LLM decides
- Recommended skills/agents are guidance; agent retains full autonomy
- Ward recommendation includes directory structure for domain-level workspaces

## System Prompt Architecture

The system prompt is assembled from modular config files at `~/Documents/zbot/config/`. Each file is created from an embedded starter template on first run and is user-customizable. Assembly is handled by `gateway/gateway-templates/src/lib.rs`.

```
┌─────────────────────────────────────────┐
│ SOUL.md — Agent identity/personality    │
│                                         │
│ Who the agent is, its personality...    │
├─────────────────────────────────────────┤
│ INSTRUCTIONS.md — Execution rules       │
│                                         │
│ How the agent should behave...          │
├─────────────────────────────────────────┤
│ OS.md — Platform-specific commands      │
│ (auto-generated for current OS)         │
│                                         │
│ - Windows: PowerShell/cmd syntax        │
│ - macOS: Unix shell + brew              │
│ - Linux: Unix shell + package managers  │
├─────────────────────────────────────────┤
│ # --- SYSTEM SHARDS ---                 │
├─────────────────────────────────────────┤
│ tooling_skills.md (shard)               │
│ - Skills-first approach                 │
│ - Delegation patterns                   │
├─────────────────────────────────────────┤
│ memory_learning.md (shard)              │
│ - Shared memory usage                   │
│ - Pattern recording                     │
├─────────────────────────────────────────┤
│ planning_autonomy.md (shard)            │
│ - Planning and autonomous execution     │
├─────────────────────────────────────────┤
│ (any extra user shards in config/shards)│
└─────────────────────────────────────────┘
```

### Assembly Order

1. **`config/SOUL.md`** — Agent identity/personality (created from `soul_starter.md` if missing)
2. **`config/INSTRUCTIONS.md`** — Execution rules (created from `instructions_starter.md` if missing)
3. **`config/OS.md`** — Platform-specific commands (auto-generated for current OS if missing)
4. **Shards** — `config/shards/{name}.md` overrides embedded defaults; extra user files included too

### Shards

Required shards are loaded from `config/shards/` if present, otherwise from embedded defaults. Users can override any shard by placing a file with the same name in the shards directory.

| Shard | Purpose |
|-------|---------|
| `tooling_skills` | Skills-first approach, delegation |
| `memory_learning` | Shared memory patterns |
| `planning_autonomy` | Planning and autonomous execution |

Extra `.md` files placed in `config/shards/` are automatically included after the required shards.

### Distillation Prompt

The distillation prompt is customizable via `config/distillation_prompt.md`. If the file does not exist, the embedded default is written to disk on first run. This allows users to tune what facts, entities, and relationships are extracted during session distillation.

### Key Files

| File | Purpose |
|------|---------|
| `gateway/gateway-templates/src/lib.rs` | Prompt assembly logic |
| `gateway/templates/` | Embedded starter templates (compiled in) |
| `~/Documents/zbot/config/` | User-customizable config files |

## Connectors

Connectors are external services that receive agent responses. When an agent execution completes, z-Bot can dispatch the response to one or more configured connectors.

### Connector Flow

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Trigger       │────▶│   z-Bot         │────▶│   Connector     │
│ (Cron/API/Web)  │     │   Gateway       │     │   (Your Service)│
└─────────────────┘     └─────────────────┘     └─────────────────┘
                              │
                              │ respond_to: ["my-connector"]
                              ▼
                        ┌─────────────────┐
                        │  HTTP POST to   │
                        │  your endpoint  │
                        └─────────────────┘
```

### Transport Types

| Type | Description | Use Case |
|------|-------------|----------|
| `http` | HTTP POST to callback URL | Webhooks, external APIs |
| `cli` | Execute local command | Scripts, local integrations |

### Connector Payload

When dispatching to connectors, Gateway sends:

```json
{
  "context": {
    "session_id": "sess-abc123",
    "thread_id": null,
    "agent_id": "root",
    "timestamp": "2024-01-15T09:00:00Z"
  },
  "capability": "respond",
  "payload": {
    "message": "The agent's response text",
    "execution_id": "exec-xyz789",
    "conversation_id": "conv-abc123"
  }
}
```

### Connector API

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/connectors` | List all connectors |
| GET | `/api/connectors/:id` | Get connector by ID |
| POST | `/api/connectors` | Create connector |
| PUT | `/api/connectors/:id` | Update connector |
| DELETE | `/api/connectors/:id` | Delete connector |
| POST | `/api/connectors/:id/test` | Test connector |
| POST | `/api/connectors/:id/enable` | Enable connector |
| POST | `/api/connectors/:id/disable` | Disable connector |

## Plugins

Plugins are Node.js integrations that extend z-Bot with custom capabilities. They run as child processes communicating via STDIO transport using the Bridge Protocol.

### Plugin Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           GATEWAY                                        │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐                                                    │
│  │ PluginManager   │ ◄── Discovers, starts, stops plugins              │
│  └────────┬────────┘                                                    │
│           │                                                             │
│           ▼                                                             │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                      STDIO PLUGIN PROCESS                        │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │   │
│  │  │  Node.js    │  │  plugin.json│  │  index.js   │              │   │
│  │  │  Runtime    │  │  (manifest) │  │  (entry)    │              │   │
│  │  └─────────────┘  └─────────────┘  └──────┬──────┘              │   │
│  │                                           │                      │   │
│  │                     STDIO (newline-delimited JSON)               │   │
│  │                     stdin ◄──────────────► stdout                │   │
│  └──────────────────────────────────────────┬──────────────────────┘   │
│                                             │                          │
│           ┌─────────────────────────────────┼──────────────────────┐   │
│           │                                 │                      │   │
│           ▼                                 ▼                      ▼   │
│  ┌─────────────┐  ┌─────────────────────────────────────────────┐      │
│  │BridgeRegistry│  │        Bridge Protocol Messages             │      │
│  │(as worker)   │  │  hello, ping, outbox_item, capability_invoke│      │
│  └─────────────┘  └─────────────────────────────────────────────┘      │
└─────────────────────────────────────────────────────────────────────────┘
```

### Plugin Lifecycle

```
┌─────────────────┐
│   Discovered    │ ◄── Plugin directory scanned, plugin.json parsed
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Installing    │ ◄── npm install --production (if node_modules missing)
└────────┬────────┘     120s timeout
         │
         ▼
┌─────────────────┐
│    Starting     │ ◄── Spawn node process, wait for hello handshake
└────────┬────────┘     10s timeout
         │
         ▼
┌─────────────────┐
│     Running     │ ◄── Heartbeat every 30s, processes messages
└────────┬────────┘
         │
         ├──────────────────┐
         │                  │
         ▼                  ▼
┌─────────────────┐  ┌─────────────────┐
│     Stopped     │  │     Failed      │
└─────────────────┘  └─────────────────┘
         │                  │
         │                  │ (if auto_restart)
         │                  ▼
         │          ┌─────────────────┐
         └─────────►│ restart_delay_ms│
                    └────────┬────────┘
                             │
                             ▼
                    ┌─────────────────┐
                    │    Starting     │
                    └─────────────────┘
```

### Plugin Manifest (plugin.json)

```json
{
  "id": "slackbot",
  "name": "Slack Bot",
  "version": "1.0.0",
  "description": "Slack integration plugin",
  "entry": "index.js",
  "enabled": true,
  "env": {
    "SLACK_TOKEN": "${SLACK_BOT_TOKEN}"
  },
  "auto_restart": true,
  "restart_delay_ms": 5000
}
```

### Plugin User Configuration

Stored in `plugins/{plugin_id}/.config.json` (self-contained with plugin):

```json
{
  "enabled": true,
  "settings": {
    "default_channel": "#general"
  },
  "secrets": {
    "bot_token": "xoxb-..."
  }
}
```

- Auto-created when plugin is discovered
- 0600 file permissions on Unix (owner-only)
- Deleted when plugin directory is removed

### Plugin Protocol (Bridge Protocol)

Plugins use the same protocol as Bridge Workers:

**From Plugin (stdout):**
| Message | Description |
|---------|-------------|
| `hello` | Register with adapter_id, capabilities, resources |
| `pong` | Heartbeat response |
| `ack/fail` | Outbox delivery confirmation |
| `resource_response` | Query response |
| `capability_response` | Invocation result |
| `inbound` | Send message to trigger agent |

**To Plugin (stdin):**
| Message | Description |
|---------|-------------|
| `hello_ack` | Registration confirmed |
| `ping` | Heartbeat check |
| `outbox_item` | Push message for delivery |
| `resource_query` | Query a resource |
| `capability_invoke` | Invoke a capability |

### Plugin HTTP API

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/plugins` | List all plugins with status |
| GET | `/api/plugins/:id` | Get plugin details |
| POST | `/api/plugins/:id/start` | Start a plugin |
| POST | `/api/plugins/:id/stop` | Stop a plugin |
| POST | `/api/plugins/:id/restart` | Restart a plugin |
| POST | `/api/plugins/discover` | Re-scan plugins directory |
| **Configuration** | | |
| GET | `/api/plugins/:id/config` | Get plugin configuration |
| PUT | `/api/plugins/:id/config` | Update plugin configuration |
| GET | `/api/plugins/:id/secrets` | List secret keys |
| PUT | `/api/plugins/:id/secrets/:key` | Set a secret value |
| DELETE | `/api/plugins/:id/secrets/:key` | Delete a secret |

### Implementation Files

| File | Purpose |
|------|---------|
| `gateway-bridge/src/plugin_config.rs` | PluginConfig, PluginError, PluginState, PluginSummary |
| `gateway-bridge/src/stdio_plugin.rs` | Process spawn, npm install, message framing |
| `gateway-bridge/src/plugin_manager.rs` | Discovery, lifecycle management |
| `gateway-services/src/plugin_service.rs` | Config loading, settings/secrets |
| `gateway/src/http/plugins.rs` | HTTP API endpoints |
| `plugins/.example/` | Reference plugin implementation |
| `plugins/slack/` | Slack Socket Mode integration |

## Cron Scheduler

Built-in scheduler that triggers agents on a schedule. Cron jobs always route to the **root agent** for orchestration.

### Cron Configuration

```json
{
  "id": "daily-report",
  "name": "Daily Report Generator",
  "schedule": "0 0 9 * * *",
  "message": "Generate the daily sales report",
  "respond_to": ["slack-notifier"],
  "enabled": true
}
```

**Note**: Schedule uses 6-field cron format: `sec min hour day month weekday`

### Cron API

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/cron` | List all cron jobs |
| GET | `/api/cron/:id` | Get cron job by ID |
| POST | `/api/cron` | Create cron job |
| PUT | `/api/cron/:id` | Update cron job |
| DELETE | `/api/cron/:id` | Delete cron job |
| POST | `/api/cron/:id/trigger` | Manually trigger job |
| POST | `/api/cron/:id/enable` | Enable job |
| POST | `/api/cron/:id/disable` | Disable job |

## Response Routing

The `respond_to` field controls where agent responses are delivered:

```json
{
  "agent_id": "root",
  "message": "Generate a report",
  "respond_to": ["slack-notifier", "email-bridge"]
}
```

- **Empty/null**: Response goes to web UI only (default)
- **Specified**: Response dispatched to listed connectors
- **Original source NOT automatically included** (explicit routing)

## UI: Observatory and Execution Intelligence Dashboard

### Observatory (Knowledge Graph)

D3-force directed graph visualization of the knowledge graph. Entity detail sidebar on click. Learning health bar shows distillation success rate with a backfill button for retroactive distillation.

Implementation: `apps/ui/src/features/observatory/`

### Mission Control

The flat log viewer and the WebOps/Executions dashboard (KPI cards, mini waterfalls, slide-out panels) were retired in PR #112. The current observability surface lives at the live route `/mission-control`; `/logs` and `/dashboard` redirect there.

- **KpiStrip** — compact aggregate counters (sessions, tokens, tool calls, duration)
- **SessionListPanel** — filterable list of root sessions with status badges and real-time updates
- **SessionDetailPane** — selected session detail with `MessagesPane` (chat narrative) and `ToolsPane` (`AgentToolGroup` + `ToolDetailPopover` for per-tool inspection)

Implementation: `apps/ui/src/features/mission-control/`

## Extension Points

### GraphTraversal Trait

Abstract graph backend — SQLite recursive CTE today, Neo4j tomorrow. The trait provides `expand_from_entity(entity_id, max_hops)` for recall graph expansion and `find_related(entity_ids, relationship_types)` for targeted traversal.

Implementation: `services/knowledge-graph/src/traversal.rs`

### New CLI Commands

| Command | Description |
|---------|-------------|
| `zbot distill backfill` | Retroactive distillation for sessions that pre-date the pipeline |
| `zbot sessions archive --older-than 7` | Offload old transcripts to JSONL.gz |
| `zbot sessions restore <session_id>` | Restore an archived session |

## Runtime Memory Profile

Typical daemon (`zbotd`) memory usage: **~150 MB** at idle after first request.

### Breakdown

| Component | Approx. Size | Source |
|-----------|-------------|--------|
| **fastembed ONNX model** | ~100 MB | `AllMiniLmL6V2` model loaded at startup for local embeddings. Held in `EmbeddingClient` inside `AppState`. |
| **SQLite connection pool** | ~32–64 MB | r2d2 pool with `max_size(8)` connections, each configured with `PRAGMA cache_size = -8000` (8 MB per connection). |
| **Service caches** | ~5–10 MB | `AgentCache` (RwLock), `TemplateCache`, `ConnectorRegistry`, `BridgeRegistry` — all in-memory hashmaps. |
| **Tokio runtime + stacks** | ~2–5 MB | Multi-threaded runtime, green thread stacks, channel buffers. |
| **Base process** | ~5–10 MB | Executable code, static data, Rust allocator overhead. |

### Key Configuration Points

| Setting | Value | File | Impact |
|---------|-------|------|--------|
| SQLite `cache_size` | System-profiled | `stores/zbot-stores-sqlite/src/connection.rs`, `stores/zbot-stores-sqlite/src/knowledge_db.rs` | Per-connection page cache. Multiply by pool size. |
| Pool `max_size` | System-profiled | `stores/zbot-stores-sqlite/src/connection.rs`, `stores/zbot-stores-sqlite/src/knowledge_db.rs` | Number of SQLite connections kept open. |
| Embedding model | `AllMiniLmL6V2` | `runtime/agent-runtime/src/llm/embedding.rs` | ~100 MB ONNX model. Switch to provider-based embeddings (`EmbeddingConfig::Provider`) to eliminate. |
| BatchWriter flush | `100ms` | `gateway/gateway-execution/src/invoke/batch_writer.rs` | Batches inserts; small buffer (~KB). |
| BridgeRegistry | Unbounded `HashMap` | `gateway/gateway-bridge/src/registry.rs` | Grows with connected workers; negligible at typical scale. |

### Optimization Levers

- **Disable local embeddings**: Set `EmbeddingConfig::Provider` to offload to an API — saves ~100 MB
- **Reduce pool size**: Lower `max_size` to 4 — saves ~32 MB (trades throughput under load)
- **Reduce cache_size**: Set `PRAGMA cache_size = -4000` — saves ~4 MB per connection
- **Lazy model loading**: Defer fastembed init until first `recall`/`save_fact` — saves startup RAM if memory features unused
