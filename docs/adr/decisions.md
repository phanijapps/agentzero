# z-Bot — Decisions

## Technology Choices

### Why Rust
- Memory safety without GC
- Excellent async story (tokio)
- Great tooling (cargo, clippy)
- Single binary distribution

### Why SQLite + WAL Mode + r2d2 Pool
- Zero configuration, portable (single file), ACID transactions
- WAL mode enables concurrent readers with single writer
- r2d2 pool (max 8, min idle 2) replaces single Mutex — eliminates serialization bottleneck
- Sync rusqlite + pool chosen over async sqlx: simpler, DB operations are simple enough that sync + pool is fine
- PragmaCustomizer applies WAL/perf pragmas to every pooled connection

### Why Single Daemon
- Simpler deployment and debugging
- Shared state without IPC complexity
- Single port configuration
- Memory efficiency

### Why No Desktop Wrapper
- Browsers are more capable than custom webviews
- Easier deployment (no native installers)
- Better developer experience (standard web tools)
- Cross-platform without platform-specific builds

### Why React + Vite
- React 19 for UI components
- Vite for fast dev server and bundling
- Tailwind CSS v4 + Radix UI for styling and accessible primitives

### Why Instructions in AGENTS.md
- Human-readable and editable
- Version control friendly
- Markdown rendering in UI
- Separates behavior from configuration

## Architecture Decisions

### Tool Tiers (Core, Optional, Action) — Shell + Dedicated File Tools
- **Core**: shell, write_file, edit_file, memory, ward, update_plan, set_session_title, execution_graph, list_skills, load_skill, grep — always available
- **Action (3)**: respond, delegate_to_agent, list_agents — always available, drive agent behavior
- **Optional**: read, write, edit, glob, todos, python, web_fetch, ui_tools, create_agent, introspection — configurable per agent
- File writes go through `write_file` (create) and `edit_file` (targeted edits). The `shell` tool rejects file-writing commands (`cat >`, heredocs, `Set-Content`, etc.) to force use of the dedicated tools.
- Old standalone knowledge_graph tools (5) removed — replaced by unified `graph` action in memory tool

### Memory: 4-Tier Hierarchy
| Tier | Path | Purpose |
|------|------|---------|
| Global Shared | `agents_data/shared/*.json` | user_info, workspace, patterns, session_summaries |
| Agent | `agents_data/{agent_id}/memory.json` | Per-agent private context |
| Ward | `wards/{ward_id}/AGENTS.md` | Project-specific context (ward memory) |
| Session | `agent_data/{session_id}/` | Ephemeral: attachments, scratchpad |

File locking (fs2 crate) protects shared memory from concurrent access.

### Code Wards Over Session-Scoped Dirs
**Problem**: Per-session `code/sess-{uuid}/` directories were isolated and ephemeral — no project continuity.
**Decision**: Named persistent directories (wards) that agents create, name, and navigate autonomously.
**Key principles**: Agent autonomy (agent decides ward name/location), shared Python venv across all wards, per-ward node_modules (Node convention), simplicity (wards are directories, no metadata database).

### Agent Autonomy in Ward Selection
The agent — not the user — decides which ward to work in. System prompt instructs: list wards, match to task, create new one if needed, use `scratch` for one-offs.

### Gateway Crate Decomposition (13 Crates)
Gateway was a 73-file monolith (~15,675 LOC). Extracted into focused crates:
- `gateway-events` — EventBus, GatewayEvent, HookContext
- `stores/zbot-stores-sqlite` — DatabaseManager, pool, schema, ConversationRepository
- `gateway-templates` — Prompt assembly, shard injection
- `gateway-connectors` — ConnectorRegistry, dispatch
- `gateway-services` — AgentService, ProviderService, McpService, SkillService, SettingsService
- `gateway-execution` — ExecutionRunner, delegation, lifecycle, streaming, BatchWriter
- `gateway-hooks` — Hook trait, HookRegistry, CliHook, CronHook
- `gateway-cron` — CronJobConfig, CronService
- `gateway-bus` — GatewayBus trait, SessionRequest, SessionHandle
- `gateway-ws-protocol` — ClientMessage, ServerMessage, SubscriptionScope
- `gateway/src/` — Thin shell: HTTP routes, WebSocket handler, AppState

Dependency direction: events ← database ← services ← execution ← gateway (thin shell). No reverse dependencies.

### Parallel Tool Execution (Always Parallel)
When the LLM returns multiple tool calls, they all execute concurrently via `futures::future::join_all`. Sequential loop replaced with concurrent execution. Events emitted in original order (start upfront, results in order). `take_actions()` atomic capture prevents race conditions.

### Real Streaming (mpsc Channel Bridge)
5ms/char simulation removed. Tokens stream in real-time from LLM to UI via mpsc channel bridge. Intermediate text alongside tool calls now visible.

### Batch DB Writes (BatchWriter)
Background task decouples DB writes from streaming. Token updates coalesced by execution_id, logs batched. Flushes every 100ms or when 10+ items queued. Prevents hot-path DB writes from blocking stream processing.

### RwLock Service Caching
ProviderService, McpService, and SettingsService use RwLock caches (providers.json, mcps.json, settings.json). Reads don't block each other. Cache invalidated on writes.

### AGENTS.md as Documentation Standard
Every crate directory has an AGENTS.md describing what it does, its key files, and what it does NOT handle. These serve as both human documentation and AI context.

### Memory Evolution: Brute-Force Cosine Over sqlite-vec
**Problem**: Hybrid search needs vector similarity. sqlite-vec requires loading a native extension (.dll/.so) at runtime — platform-specific, tricky distribution.
**Decision**: Compute cosine similarity in Rust. Load all embeddings from `embedding_cache` for the agent, compute similarity in-memory, take top-K.
**Rationale**: For <10K facts, brute-force is ~2-5ms. No extension loading, no platform concerns, no ANN index needed. Revisit if facts exceed 100K.

### Memory Evolution: Local Embeddings as Default
**Problem**: Embedding APIs cost money and require internet. Not all users have API keys configured.
**Decision**: Default to `fastembed` crate with `all-MiniLM-L6-v2` (384 dimensions, ~100MB ONNX model). Local ONNX inference on CPU — zero API calls, zero cost, works offline.
**Alternative**: OpenAI-compatible API (configurable per provider). User can switch to Ollama `nomic-embed-text` (768d) or OpenAI `text-embedding-3-small` (1536d) via `EmbeddingConfig`.

### Memory Evolution: Session Distillation Over Manual Memory
**Problem**: Agents forget to save important facts. Users shouldn't have to remind them.
**Decision**: After each session (>10 messages), fire-and-forget LLM call extracts durable facts (preferences, decisions, patterns, entities, instructions, corrections) into `memory_facts` with embeddings.
**Advantage**: We have full session transcripts (Session Tree).
**Safety**: Fire-and-forget (never blocks user), async spawn, only runs on sessions with sufficient signal (>10 messages).

### Memory Evolution: Hybrid Search Scoring
**Formula**: `(0.7 × vector_cosine + 0.3 × BM25_score) × confidence × recency_decay × mention_boost`
**Rationale**: Vector search handles semantic similarity ("preferred language" matches "coding in Rust"), FTS5 handles exact keyword matches ("SQLite" matches "SQLite"). Confidence, recency, and mention_count prevent stale or low-quality facts from dominating.

### Memory Evolution: Embedding Cache (Hash-Based Dedup)
**Problem**: Re-embedding unchanged content wastes API calls or compute time.
**Decision**: SHA-256 hash of text + model name as composite key in `embedding_cache` table. Before embedding, check cache.

### Memory Evolution: Pre-Compaction Memory Flush
**Problem**: When context window is trimmed, the agent loses access to potentially important information.
**Decision**: Before compaction, inject a `[MEMORY FLUSH]` system message warning the agent. Skip compaction for one turn to give the agent a chance to `save_fact`. On the next trigger, proceed with normal compaction.

### Knowledge Graph: Unified in Memory Tool
**Problem**: 5 standalone knowledge_graph tools cluttered the tool registry and duplicated the memory concept.
**Decision**: Remove standalone tools. Add `graph` action to existing memory tool. Entities and relationships are automatically extracted during session distillation — no manual management needed. The `graph` action provides query access when needed.

### Memory UI: Cross-Agent View by Default
**Problem**: Memory UI required selecting an agent first, showing "No memories found" even when memories existed.
**Decision**: Add `/api/memory` endpoint that lists ALL memories across all agents. UI shows all memories by default with optional agent filter. Stats computed from all memories. Agent badge shown when viewing across all agents.

### TriggerSource: Unified Session Origin Tracking
**Problem**: Sessions tracked their source (`web`, `cli`, `api`, `cron`, `connector`) but this wasn't documented for integration developers.
**Decision**: Document all invocation methods and their source values in architecture.md. Web sessions stay open for interactive use; CLI/Cron/API/Connector sessions auto-complete after execution. Source displayed in UI with badges and icons.

### Bridge Workers: STDIO Transport (Planned)
**Problem**: Bridge workers only support WebSocket transport. Some integration scenarios (embedded, local) benefit from subprocess STDIO communication.
**Decision**: Add STDIO transport following MCP server pattern. Gateway spawns worker processes configured in `bridge_workers.json`, communicates via newline-delimited JSON over stdin/stdout. Same protocol as WebSocket, different framing. Cross-platform (Windows, Linux, macOS).

### Plugins: STDIO with Bridge Protocol Reuse
**Problem**: Users want custom integrations (Slack, Discord, Strava) without modifying core codebase.
**Decision**: Node.js plugins in `~/Documents/zbot/plugins/`, spawned as child processes with STDIO transport. Reuse Bridge Protocol (hello/handshake, ping/pong, capability_invoke, inbound) — plugins appear as bridge workers to agents.
**Rationale**: Single protocol for all external integrations. Plugins can trigger agents via `inbound` messages and respond via `capability_invoke`. npm install on first start handles dependencies automatically.

### Plugins: Self-Contained Manifests
**Problem**: Configuration scattered across multiple files complicates plugin distribution.
**Decision**: Each plugin is a self-contained directory with `plugin.json` manifest (id, name, entry, env, auto_restart), `package.json` for npm dependencies, and `index.js` entry point.
**Rationale**: Drop-in deployment — copy directory, restart daemon, plugin auto-discovered and started. No central registry file to edit.

### Plugins: Per-Plugin User Configuration
**Problem**: Secrets (API tokens) shouldn't be in plugin manifests (checked into version control). Also, config orphaned when plugin deleted.
**Decision**: User config stored in `plugins/{plugin_id}/.config.json` — self-contained with plugin. Contains `settings` (non-sensitive) and `secrets` (masked in API responses, 0600 file permissions on Unix). Auto-created on discovery, deleted with plugin directory.
**Rationale**: Plugin authors ship `plugin.json` with env var references (`${SLACK_TOKEN}`); users set secrets via API. Dot-prefix (`config.json`) clearly separates user data from plugin code. Drop-in deployment — delete directory = clean uninstall.

### Plugins: Auto-Restart with Backoff
**Problem**: External services (Slack, Discord) have transient connection issues; plugins shouldn't require manual intervention.
**Decision**: `auto_restart: true` (default) with configurable `restart_delay_ms` (default 5000). Failed plugins restart automatically after delay.
**Rationale**: Hands-off operation. For intentional stops, auto_restart is skipped. Log messages indicate restart reason.

### Resource Indexing: Lazy On-Demand
**Problem**: Skills and agents need semantic search, but scanning directories at startup is wasteful.
**Decision**: Indexing happens on-demand when `index_resources` tool is called. First discovery falls back to disk scan if no index exists.
**Rationale**: Avoids startup overhead. Users/agents explicitly trigger reindex when they know files have changed. Mtime tracking enables staleness detection without full reindex.

### Resource Indexing: Dual Storage (Memory + Graph)
**Problem**: Semantic search needs text embeddings, but relationship queries need structured graph data.
**Decision**: Store skills/agents in both MemoryFactStore (for semantic search via BM25 + embeddings) and KnowledgeGraphStore (for entity relationships).
**Rationale**: Each storage is optimized for its access pattern. Memory provides hybrid search. Graph provides relationship traversal. Storing in both enables rich discovery without sacrificing performance.

### Intent Analysis: Autonomous Middleware (Not a Tool, Not Pre-Collected Arrays)
**Problem**: Earlier versions tried two approaches that both failed: (1) a tool the agent was supposed to call (but it was never registered), and (2) pre-collecting all resources into arrays to pass to the LLM (wasted tokens on irrelevant resources).
**Decision**: Intent analysis is an autonomous middleware that indexes resources into `memory_facts` with local embeddings, performs semantic search, and sends only top-N relevant resources to the LLM. Not a tool agents call. Not a full catalog dump.
**Rationale**: Autonomous indexing + semantic search means the LLM only sees relevant resources (8 skills, 5 agents, 5 wards max). No wasted tokens. No registration issues. Middleware runs before the root agent's first LLM call — transparent and automatic. See `docs/architecture/components/intent-analysis/overview.md` for full documentation.

### Modular System Prompt Config (SOUL.md, OS.md, Overridable Shards)
**Problem**: A single `INSTRUCTIONS.md` file mixed identity, platform commands, and behavior rules. No way to override individual shards without forking the embedded defaults.
**Decision**: Split system prompt into modular config files at `~/Documents/zbot/config/`:
- `SOUL.md` — agent identity/personality
- `INSTRUCTIONS.md` — execution rules
- `OS.md` — auto-generated platform-specific commands (Windows/Mac/Linux)
- `shards/` — overridable shards (defaults written to disk on first run)
- `distillation_prompt.md` — customizable distillation prompt
**Rationale**: Users can customize any piece independently. OS-specific commands are auto-generated (no manual maintenance). Shards can be overridden by placing a file with the same name in `config/shards/`. Extra user shards are automatically included. Assembly: `gateway/gateway-templates/src/lib.rs`.

### ThrottledLlmClient: Per-Provider Concurrent Request Limiting
**Problem**: Multiple concurrent delegations + root agent can burst many simultaneous LLM calls to the same provider, triggering 429 rate limits.
**Decision**: Wrap LLM clients with `ThrottledLlmClient` that uses a shared `tokio::sync::Semaphore` per provider. All executors sharing the same provider share the semaphore.
**Rationale**: Simple, composable. Configured via `maxConcurrentRequests` in `providers.json` (default: 3). Wrapping chain: `OpenAiClient -> RetryingLlmClient -> ThrottledLlmClient`. Implementation: `runtime/agent-runtime/src/llm/throttle.rs`.

### Model Configuration: Defaults and Overrides Over Catalogs
**Problem**: Maintaining model context windows and capability metadata in a bundled/local catalog created stale configuration pressure and blocked simple custom-model usage.
**Decision**: Remove the bundled/local `models.json` catalog. Unknown/custom models use 200k input and 32k output defaults. Provider `modelConfigs`, agent settings, and Advanced settings own explicit overrides.
**Rationale**: Simple, user-editable, and compatible with OpenAI-style providers where model IDs change faster than bundled releases. Implementation: `gateway/gateway-services/src/models.rs`, `gateway/gateway-services/src/providers.rs`, `gateway/gateway-execution/src/invoke/executor.rs`.

### Provider Default Model: Explicit Over Positional
**Problem**: Default model was `provider.models[0]` — an implicit positional convention. Reordering the models array silently changed which model every agent used.
**Decision**: Added `defaultModel` field to Provider struct. Priority: explicit `defaultModel` > first in `models` array > `"gpt-4o"` fallback.
**Rationale**: Explicit is always better than implicit. Users can set their preferred default without worrying about array order. Implementation: `gateway/gateway-services/src/providers.rs`.

### Settings UI: Deprecate Optional Tool Toggles
**Problem**: Optional tool toggles (python, webFetch, todos, fileTools, uiTools, createAgent) confused non-technical users. Most didn't work correctly — TS types referenced non-existent backend keys (`grep`, `glob`, `loadSkill`). The toggles gave a false sense of control.
**Decision**: Remove the Optional Tools section from the Settings UI. Keep introspection enabled via `settings.json`. Backend `ToolSettings` struct unchanged — tools can still be enabled programmatically or via direct JSON editing. UI focuses on the two settings that actually matter: context protection (offload) and logging.
**Rationale**: Simpler surface for non-technical users. The tools that matter (shell, write_file, edit_file, memory, ward) are always on. The deprecated toggles were rarely used and poorly implemented.

### Providers Page: Card Grid Over Split Panel
**Problem**: The original split-panel layout (sidebar list + detail pane) was a developer tool pattern. No inline editing, no guided setup, no capability visibility. Non-technical users didn't know what to do.
**Decision**: Card grid with responsive 2-column layout. Each card shows provider name, status badge (Connected/Not tested/Active), model chips with capability badges. Click opens a slide-over detail panel with view/edit toggle. Empty state shows Top-3 preset cards (OpenAI, Anthropic, Ollama) with inline "type API key and connect" flow.
**Rationale**: Cards give visual overview. Inline connect reduces the happy path to 2 clicks. Slide-over preserves context (grid visible behind). View/Edit toggle prevents accidental changes. 9 provider presets with pre-filled baseUrl and models. Implementation: `apps/ui/src/features/settings/` (moved from integrations during UI revamp).

### UI Revamp: 7 Pages → 3 Consolidated Pages
**Problem**: UI had 7+ separate pages (Settings, Providers, MCPs, Skills, Agents, Workers, Schedules) with inconsistent layouts, no help text, and jargon-heavy labels. Non-technical users couldn't navigate.
**Decision**: Consolidate into 3 tabbed pages:
- **Settings** (Providers | General | Logging) — system setup, providers as default tab
- **Agents** (My Agents | Skills Library | Schedules) — agent management with card grid
- **Integrations** (Tool Servers | Plugins & Workers) — MCPs renamed to "Tool Servers", workers+plugins unified
**Rationale**: Maps to user mental model: Setup → Build → Connect. Tabs reduce navigation decisions. Every section gets contextual help text and rich empty states for onboarding. Card grid + slide-over pattern used consistently across all pages.

### UI Revamp: Design System — Warm Editorial Palette
**Problem**: Existing UI used generic system fonts and basic colors. No distinctive visual identity.
**Decision**: Warm copper accent (`#c8956c` dark, `#a07d52` light), cream/charcoal backgrounds, sidebar always dark in both themes. CSS custom properties for full themability. Backwards-compat aliases for all replaced tokens during migration.
**Rationale**: User preferred existing font stack over Fraunces serif for headings. Sidebar-always-dark provides visual anchor. Token aliases prevent silent CSS breakage. Implementation: `apps/ui/src/styles/theme.css`, `components.css`.

### Plugins vs Workers: Separate APIs, Shared Runtime
**Problem**: Plugins (auto-discovered from `~/Documents/zbot/plugins/`) and bridge workers (WebSocket-connected) both appear in `/api/bridge/workers` when running, causing the UI to show plugins as workers.
**Decision**: Use `/api/plugins` as source of truth for plugin listing (state, version, auto_restart, config). Enrich with bridge worker data (capabilities, resources) when plugin is connected. Filter bridge workers list to exclude entries matching known plugin IDs.
**Rationale**: Plugin `adapter_id` equals plugin `id` (e.g. `"slack"`, NOT `"plugin:slack"`). A running plugin appears in both APIs. UI must deduplicate. Implementation: `apps/ui/src/features/integrations/WebIntegrationsPanel.tsx`.

### Score Threshold + Per-Category Caps for Semantic Search
**Problem**: Sending all indexed resources to the LLM wastes tokens on irrelevant results and produces noisy recommendations.
**Decision**: Apply a minimum relevance score threshold (0.15) and per-category caps (8 skills, 5 agents, 5 wards) when selecting resources for intent analysis.
**Rationale**: Recall 50 results from `memory_facts`, filter by score, cap per category. LLM sees only relevant resources. Thresholds tuned empirically — 0.15 filters noise without losing useful matches.

### Child Session Lifecycle Fix
**Problem**: Delegated subagent sessions were left in `running` state after completion, creating orphan sessions.
**Decision**: Child sessions are now explicitly marked `completed` when the subagent finishes execution.
**Rationale**: Clean lifecycle prevents orphaned sessions from accumulating in the database. Combined with delegation semaphore (max 3 concurrent) to prevent resource exhaustion.

### Session Distillation: Lower Threshold, Broader Trigger
**Problem**: Distillation only fired after `invoke()` and required 10+ messages, missing useful short sessions and continuation sessions.
**Decision**: Fire distillation after both `invoke()` and `invoke_continuation()`. Lower min messages threshold from 10 to 4.
**Rationale**: Many useful sessions are short (4-6 messages). Continuation sessions often contain important follow-up decisions. Distillation prompt is customizable via `config/distillation_prompt.md`.

### Recall: Tool-Call Based, Not Hidden Injection
**Problem**: Hidden memory injection is invisible — agents can't learn when or what to recall, and developers can't debug what was injected.
**Decision**: Recall is an explicit tool call (`memory recall`). The agent decides when and what to recall. Results appear in the conversation as tool output.
**Rationale**: Visible in transcripts, debuggable in logs, learnable by the agent (it sees what works). Nudges at session start, ward entry, and post-delegation prompt the agent to recall without forcing it.

### Corrections as Rules (NEVER/ALWAYS)
**Problem**: LLMs treat "correction: don't do X" as a suggestion. They follow "NEVER do X" more reliably.
**Decision**: Top correction facts are formatted as imperative rules ("NEVER use `rm -rf` without confirmation", "ALWAYS check ward before file operations") and injected first in recall results.
**Rationale**: Rule-formatted corrections have higher compliance in practice. Filtered by query relevance so only applicable corrections appear.

### Graph Traversal via SQLite Recursive CTE
**Problem**: Knowledge graph expansion for recall needs graph traversal. Neo4j is the standard but adds an external dependency.
**Decision**: 2-hop BFS via SQLite recursive CTE behind a `GraphTraversal` trait. Trait-based so Neo4j can be swapped in later.
**Rationale**: Pi 4 safe (<10ms for 2-hop expansion), zero extra dependencies. SQLite CTE handles the current graph size (198+ entities, 333+ relationships) easily. Neo4j swap is a trait implementation change, not a redesign.

### Temporal Decay with Per-Category Half-Lives
**Problem**: Not all facts stale at the same rate. Domain knowledge ("project uses React 19") stales faster than corrections ("NEVER skip tests").
**Decision**: Per-category half-lives configured in `recall_config.json`: domain facts (30d), preferences (60d), corrections (90d), strategies (90d). Facts past their half-life decay exponentially in recall scoring; past 2x half-life they move to `memory_facts_archive`.
**Rationale**: Keeps recall fresh without losing corrections. Archive preserves audit trail.

### Session Offload to JSONL.gz
**Problem**: Session transcripts are the largest data in SQLite. After distillation, raw transcripts are dead weight.
**Decision**: `zbot sessions archive --older-than N` compresses transcripts to JSONL.gz files. `sessions.archived` column tracks state. `zbot sessions restore <id>` reverses the process.
**Rationale**: Keeps SQLite lean for Pi 4. Transcripts are already distilled — the extracted facts, episodes, and entities survive independently. Restorable if needed.

### Entity Dedup as __global__
**Problem**: The same entity (e.g., "React") was duplicated per agent — "React" x4 across root, researcher, coder, reviewer.
**Decision**: Entities are stored under `__global__` agent scope. Cross-agent dedup during distillation merges identical entities.
**Rationale**: Single source of truth. Graph queries return one "React" node with all relationships, not four disconnected copies.

### Execution Intelligence Dashboard Over Flat Logs
**Problem**: The log viewer was 845 lines of flat session/log rendering. No visual structure, no execution shape, no interactivity.
**Decision**: A visual observability surface focused on execution shape — aggregate KPIs, a filterable session list, and a per-session detail pane that shows the message narrative alongside the tool calls each agent issued. Auto-refresh while sessions are running.
**Rationale**: Operators need to see execution shape at a glance — which delegations happened, where time was spent, what tools were called. Flat logs require scrolling and mental reconstruction.
**Implementation refs**: `apps/ui/src/features/mission-control/` — `MissionControlPage`, `KpiStrip`, `SessionListPanel`, `SessionDetailPane`, `MessagesPane`, `ToolsPane`. The earlier WebOps/Executions implementation (KPI sparkline cards, mini waterfalls, `WaterfallSlideOut`) was retired in PR #112 — same decision, simpler surface.

## Patterns We Did NOT Adopt

These were considered during the Codex gap analysis and explicitly rejected:

| Pattern | Why Skipped |
|---------|-------------|
| Platform-native sandboxing | z-Bot is a web platform, not a CLI. Users explicitly configure agent tools. Different threat model. |
| Interactive approval (exec policy) | Would require WebSocket round-trips + UI modals. z-Bot agents run in background. Tool tiers are sufficient. |
| TOML config hierarchy (7 layers) | z-Bot has a single config directory per installation. JSON/YAML is simpler and sufficient. |
| OpenTelemetry | `tracing` crate is sufficient. OTEL adds infrastructure requirements not needed at current scale. |
| Git ghost commits (undo) | Different UX for web platform vs interactive CLI. Not critical path. |
| Starlark policy language | Over-engineered. Simple JSON allowlists are sufficient. |
| sqlx compile-time query checking | Overkill for AZ's current schema complexity. |

## Development Workflow

Distilled patterns from building z-Bot:

1. **Plans with concrete data models** — Show actual schemas/types, not prose. File-level specificity. Phase grouping by architectural layer.
2. **Right level of detail** — Specify what to create, not how. File paths guide but don't micromanage. Assumes competence on implementation details.
3. **Layer-by-layer implementation** — Follow the dependency graph: `runtime/ → stores/ → services/ → gateway/ → apps/`. Backend before frontend.
4. **Test each phase** — `cargo check --workspace` after Rust changes, `npm run build` after TypeScript. Don't batch all testing at the end.
5. **Read before write** — Check existing patterns, understand current state, avoid duplicating functionality.
6. **Anti-patterns**: Starting without reading context. Solving problems not asked. Ignoring existing patterns. Plans without data models. Skipping root cause analysis.

## Setup Wizard Decisions

### Dedicated Route Over Settings Enhancement
**Problem**: New users land on an empty app with no guidance.
**Decision**: Full-page `/setup` wizard at a dedicated route, outside the app shell (no sidebar). Not a modal inside Settings.
**Rationale**: The wizard is a distinct user journey with its own lifecycle. Coupling it to Settings would make both harder to maintain. The wizard calls the same transport APIs — it just has its own UI flow.

### Hybrid Trigger (Auto-Redirect + Manual Re-Run)
**Problem**: First-time users need guidance, but the wizard should also be accessible later.
**Decision**: Auto-redirect to `/setup` if no providers AND `setupComplete === false`. Re-run button in Settings > Advanced. `SetupGuard` checks `GET /api/setup/status` (lightweight) and caches result in sessionStorage.
**Rationale**: Catches new users without locking out the wizard for reconfiguration.

### Providers Persisted Immediately, Everything Else at Launch
**Problem**: Step 5 (agent config) needs real provider IDs and discovered model lists from Step 2.
**Decision**: Provider create + test happens in real-time during Step 2. All other changes (name, agent configs, MCPs) are held in React state and submitted on Launch.
**Rationale**: Model discovery requires a real API call. If user abandons mid-wizard, they keep configured providers (useful work preserved).

### Delta-Only Updates on Re-Run
**Problem**: Re-running the wizard could overwrite customized agent configs, duplicate MCPs, or reset the agent name.
**Decision**: Wizard hydrates from current state on mount. Launch only applies changes: agent configs updated only where different, MCPs created only if new, root renamed only if name changed.
**Rationale**: Users customize agents over time. A re-run that resets everything would destroy their work.

### Agent Name in settings.json + SOUL.md
**Problem**: The root agent's name needs to persist across restarts and appear in the system prompt.
**Decision**: Store `agentName` in `settings.json` (source of truth). When updated, gateway also writes it to `config/SOUL.md` first line (`You are **Name**`).
**Rationale**: settings.json is the API-accessible config. SOUL.md is what the LLM sees. Both must stay in sync.

### Bundled Agent Templates Over Hardcoded Constants
**Problem**: Only 3 agents were hardcoded in Rust. User had 7 custom agents that wouldn't ship to new installs.
**Decision**: `default_agents.json` template with all 7 agents (temps, maxTokens, skill/MCP refs). Seed function reads template, falls back to hardcoded 3 if missing.
**Rationale**: Templates are editable without recompilation. New agents added by editing JSON, not Rust code.

### Skills Not Bundled
**Problem**: 28 skills with scripts, assets, and ONNX models are too large for binary embedding.
**Decision**: Skills remain disk-only. Wizard shows "No skills installed" on fresh install. Users install skills separately.
**Rationale**: Skills have external dependencies (npm packages, Python scripts, browser binaries). Bundling would bloat the binary and create a maintenance burden. The skill ecosystem is designed for independent installation.

## Memory Brain Decisions

### FTS5 Query Sanitization
**Problem**: Raw user messages passed to FTS5 MATCH contain commas, parens, dashes that break FTS5 syntax. Multi-word queries used implicit AND — requiring ALL terms in one fact (never matches).
**Decision**: `sanitize_fts_query()` extracts alphanumeric words (>2 chars), filters stopwords, joins with OR.
**Rationale**: "portfolio risk PTON NVDA" → "portfolio OR risk OR PTON OR NVDA" matches any fact with any term. This was THE fix that unblocked the entire memory brain.

### Subagents Get WardTool + MemoryTool + GrepTool
**Problem**: Subagents (planner, code-agent, etc.) had only Shell, WriteFile, EditFile, LoadSkill, Respond. They couldn't enter wards, recall memory, or search code efficiently. Planner was planning blind.
**Decision**: Add WardTool, MemoryTool, GrepTool to ALL subagents.
**Rationale**: Every subagent benefits from ward context (AGENTS.md, core_docs), memory (corrections, strategies), and grep (find functions without cat-ing files). Read-only awareness tools have no downside.

### Fact Dedup by Content Similarity
**Problem**: Distillation creates near-duplicate facts under different keys ("user.portfolio.holdings" vs "domain.finance.portfolio_holdings"). Same content appears 3-5 times.
**Decision**: Before upserting, check if any existing fact has 60%+ word overlap with the new fact. Skip if duplicate.
**Rationale**: Better to miss a slightly-different fact than to waste recall slots on 5 copies of the same information.

### Failed Episodes as Warnings
**Problem**: 43 episodes, many failed/crashed. Agents repeated failed strategies because failures weren't surfaced.
**Decision**: Recall now has a "Warnings (past failures)" section that surfaces failed/crashed episodes BEFORE successful experiences.
**Rationale**: Knowing what NOT to do is as important as knowing what worked. Failed strategies with key_learnings prevent the same mistake twice.

### Ward.md Curated, Not Dumped
**Problem**: ward.md was auto-dumped with all distillation facts — 40+ items, 6x duplicates, 3KB noise.
**Decision**: Max 5 corrections, 3 strategies, 2 warnings. Deduped by 60% word overlap. No domain knowledge dump.
**Rationale**: ward.md is what the agent reads FIRST — it must be concise and actionable. Domain knowledge stays in memory_facts (queryable via recall).

### core_docs.md Scans All Code Files
**Problem**: core_docs.md only scanned `core/` directory. 80% of code (analysis.py, task-specific scripts) was invisible.
**Decision**: Recursive scan of ALL `.py/.js/.ts/.rs` files in the ward, excluding node_modules, .venv, __pycache__.
**Rationale**: Agents need to know what code exists ANYWHERE in the ward, not just in a conventional `core/` directory.

### Policies as Memory Facts
**Problem**: Need to inject persistent rules (e.g., "always use research-agent for factual data") without prompt changes.
**Decision**: Policies are memory facts with category=correction, confidence=1.0, ward_id=__global__, mention_count=5.
**Rationale**: Corrections have 1.5x recall weight, highest priority. Global scope means they apply everywhere. High mention_count ensures high ranking. No new tables, no code changes — just a fact with the right metadata.

### Stream Decode Fallback
**Problem**: Z.AI/GLM returns malformed HTTP chunks during streaming, causing "error decoding response body" crashes.
**Decision**: On stream error before any content emitted, silently retry as non-streaming (single JSON). On stream error after content emitted, break gracefully and return partial response.
**Rationale**: Non-streaming is more reliable (single JSON, no SSE parsing). Automatic fallback is transparent to the executor.

### Reserved Key Prefixes for User-Managed Facts
**Problem**: Distillation creates competing facts under similar keys to user-authored policies (e.g., user sets `policy.research_first`, distillation creates `correction.always_research` with overlapping content).
**Decision**: Keys starting with `policy.`, `instruction.`, or `user.profile` are reserved — distillation skips them entirely. Three protection layers: reserved prefixes (distillation skip), pinned flag (SQL content guard), content dedup (60% word overlap).
**Rationale**: User-authored policies are the source of truth for agent behavior rules. The system should learn around them, not compete with them.

### Z.AI Rate Limit Detection
**Problem**: Z.AI returns 500 with code 1234 for rate limits (not 429). Our retry logic didn't recognize it.
**Decision**: RetryingLlmClient treats error codes 1234 (network error), 1302 (rate limit), 1303 (frequency limit) as retryable. Z.AI concurrent limit set to 1 in provider config.
**Rationale**: GLM Coding Plan has a documented concurrent request limit of 1. Two simultaneous requests = instant 500.

### Orchestrator Config in settings.json (Not agents/root/)
**Problem**: Root agent was auto-created with hardcoded settings (temp 0.7, max_tokens 8192, thinking=false). No UI to configure. No config.yaml on disk.
**Decision**: Store orchestrator config in `settings.json` > `execution.orchestrator` — NOT in `agents/root/config.yaml`. Root is a system agent, not a user-created agent.
**Rationale**: Root lives in config/ alongside providers.json and settings.json. Creating agents/root/ would confuse it with specialist agents. Settings.json is the single source of truth for system config.

### Thinking Mode Default ON for Orchestrator
**Problem**: Root agent delegated without reasoning, leading to suboptimal agent selection and planning.
**Decision**: `thinkingEnabled: true` by default for the orchestrator. Extended thinking via OpenAI-compatible `{"thinking": {"type": "enabled"}}`.
**Rationale**: The orchestrator's job is to think BEFORE delegating — which agent, which ward, which approach. Thinking mode improves planning quality. Max tokens raised to 16384 to accommodate reasoning output.

### Single Action Mode Stays Hardcoded
**Problem**: Should `single_action_mode` (one tool call per LLM turn) be configurable for root?
**Decision**: Keep hardcoded `true` for root. Not exposed in UI.
**Rationale**: Architectural enforcement — root orchestrates one delegation at a time. Multiple simultaneous delegations would confuse the delegation queue, race for the semaphore, and produce unpredictable ordering. If root needs parallel delegation, it should be via the delegation semaphore (maxParallelAgents), not via parallel tool calls.

### Smart Session Resume — Subagent-Level Retry (2026-04-08)
**Problem**: When a subagent crashes (LLM 500/429 errors), hitting Resume restarts from the root agent, re-evaluating intent, re-planning, and re-delegating all subagents — wasting tokens and duplicating completed work.
**Decision**: Resume detects the most recently crashed subagent execution via `child_session_id` on `agent_executions`, and re-spawns only that subagent using its child session's message history. Root agent stays in `running` state waiting for the retried subagent's callback.
**Rationale**: All the infrastructure exists — child sessions have full message history, the delegation completion flow (`complete_delegation` → `SessionContinuationReady`) handles the callback. Only the resume entry point needed to be smarter.
**Scope limitation**: If multiple subagents crash (parallel delegations), only the most recently started one is retried. This is acceptable since delegations are sequential per-session today.

### Distillation Config Promoted to Own Settings Card (2026-04-08)
**Problem**: Distillation model settings were buried inside the Orchestrator card on Settings > Advanced, and a backend bug (`UpdateExecutionSettingsRequest` missing `distillation` field) silently dropped the config on save.
**Decision**: Fixed the backend API to persist distillation config. Promoted Distillation to its own card in a 2x2 grid layout on the Advanced tab, alongside Orchestrator, Multimodal, and Execution.
**Rationale**: Distillation (memory extraction) is a distinct subsystem from the orchestrator. Users may want a cheaper model for distillation while keeping a powerful orchestrator model. Separate card makes it discoverable and independently configurable.

### Observability Dashboard Replaces Logs Page (2026-04-08, surface superseded 2026-05 by PR #112)
**Problem**: The Logs page used a waterfall visualization that didn't show subagent tool calls. Users couldn't see the full execution narrative (root → subagent → tool calls).
**Decision**: Replace `/logs` with a session-list + detail layout. Left panel: filterable session list. Right panel: hierarchical narrative showing root → delegation → tool calls with lucide icons per tool type. Real-time updates via polling for running sessions.
**Rationale**: All data already existed in `execution_logs` table and the `/api/logs/sessions` API. The gap was purely presentation. WebSocket `scope: "all"` delivers subagent events with `execution_id` for tagging. Polling chosen over WebSocket subscription because the subscription API uses `conversationId` (not `sessionId`) and the dashboard doesn't need sub-second latency.
**Key detail**: Delegation logs in `stream.rs` use metadata key `"child_agent"` (not `"child_agent_id"` as in `service.rs`). The trace builder checks both keys with a fallback to parsing the message text.
**Current implementation**: `/logs` (and `/dashboard`) now redirect to `/mission-control`. The list+detail split survives in `apps/ui/src/features/mission-control/` (`SessionListPanel` + `SessionDetailPane` with `MessagesPane`/`ToolsPane`). The original Timeline Tree component family (waterfall slide-outs, KPI sparkline cards) was retired in PR #112.

### Tool Result Offloading Already Exists (2026-04-08)
**Problem**: Investigated whether to build JS/shell hooks for intercepting large tool call responses.
**Decision**: No new work needed. The `offload_large_results` feature is already implemented, enabled by default, and fully wired: results > 20k chars saved to `{config_dir}/temp/`, agent receives a reference message with instructions to use `read`/`grep`.
**Rationale**: Explored the executor pipeline: offload → truncate (30k safety net) → afterToolCall hook → send to LLM. All stages working. Per-tool thresholds or external hook scripts could be future enhancements if needed.
