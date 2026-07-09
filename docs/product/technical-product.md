# z-Bot — Product Definition

z-Bot is a goal-oriented AI agent that lives on your desktop. It analyzes intent, plans autonomously, delegates to specialist subagents, self-learns across sessions, and works with any OpenAI-compatible provider. It is token-intensive by design — the agent does the work so you don't have to.

## Interfaces

### Web Dashboard
Browser-based interface served by the daemon at `http://localhost:18791`. Full-featured management of agents, providers, skills, conversations, and observability.

### CLI (zbot)
Command-line interface for scripting, automation, and terminal-based workflows. Connects to the same daemon as the web dashboard.

## Core Features

### 1. Chat Interface
Conversational interface with real-time streaming. See tool calls as they happen. Continue conversations across sessions with full history preserved in SQLite.

### 2. Agent Management
Create agents with:
- Custom system instructions (AGENTS.md files)
- Provider and model selection
- Temperature and token limits
- Skill and MCP server assignments

### 3. Provider Management
Supported providers:
- OpenAI (GPT-4, GPT-4o, etc.)
- Anthropic (Claude 3, Claude 3.5)
- DeepSeek
- Groq
- Ollama (local models)
- Any OpenAI-compatible API

Set a default provider for the root agent. Per-agent provider overrides supported.

### 4. Skill System
Reusable instruction packages following the Agent Skills specification:

```markdown
---
name: code-review
description: Reviews code for quality and bugs
category: development
---

# Code Review Skill

When reviewing code:
1. Check for security vulnerabilities
2. Identify performance issues
3. Suggest improvements
...
```

Skills are stored in `~/Documents/zbot/skills/{name}/SKILL.md`.

### 5. MCP Server Integration
Connect to external tools via Model Context Protocol servers. Configure in `mcps.json`:

```json
{
  "servers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@anthropic/mcp-server-filesystem"]
    }
  }
}
```

### 6. Memory Brain — Persistent Intelligence

The memory layer is z-Bot's cognitive system. Every session teaches it something. Agents learn from experience, avoid past mistakes, and reuse existing work. See [components/memory-layer/overview.md](components/memory-layer/overview.md) for full architecture.

**Memory Loops** (all active):
1. **System recall** — facts, episodes, knowledge-graph entities injected as system message on every first message
2. **Intent + memory** — corrections and strategies enrich intent analysis before planning
3. **Subagent priming** — corrections, skills, ward context injected when spawning subagents
4. **Mid-session refresh** — new relevant facts injected every N turns during long sessions
5. **Post-session distillation** — LLM extracts facts, entities, relationships, episodes with verification
6. **Reflective synthesis (sleep-time)** — background pipeline abstracts patterns from corrections, synthesizes beliefs from episodic clusters, resolves contradictions, and verifies pairwise entity merges

**Cognitive layers** (each adds a dimension to plain-fact storage):
- **Knowledge graph** — entities + typed relationships, 2-hop CTE expansion during recall
- **Episodic memory** — goal/outcome/tools-used records per session (`session_episodes`)
- **Bi-temporal facts** — every fact carries `valid_from` / `valid_to`; recall can be point-in-time, supersession-aware
- **Hierarchical memory (HiRAG / LeanRAG)** — clusters of related facts roll up into summaries; recall walks down from the lowest-common-ancestor cluster
- **Belief network** — multi-fact beliefs synthesized from episode clusters, with confidence propagation and contradiction graph
- **Procedures** — replayable, named runbooks the agent learned by doing; dispatchable as first-class tool calls
- **Corrections abstractor** — recurring fixes become high-priority "NEVER do X / ALWAYS do Y" rules

**Agent Tools**: All agents (root + subagents) have WardTool, MemoryTool, GrepTool, RunProcedureTool — they can enter wards, recall memory, replay procedures, and search code.

**Recall Output** (priority order):
- Rules (corrections — ALWAYS followed)
- Warnings (past failures — avoid these)
- Preferences & instructions
- Past experiences (with outcome + strategy)
- Domain knowledge

**Accuracy Layer**:
- Fact verification: distilled facts grounded against tool outputs (confidence scaled by match ratio)
- Fact dedup: 60% word overlap check prevents near-duplicates under different keys
- Entity normalization: file basename matching, alias tracking
- Relationship dedup: unique index on (source, target, type)
- Confidence-aware traversal: graph expansion weighs entity confidence; bulk decay archives stale facts

**Policies**: High-priority rules injected as memory facts (correction category, confidence 1.0, global scope). Surface at top of every recall.

**Storage**: SQLite (memory_facts, episodes, recall_log, embeddings, beliefs, hierarchy clusters) + Knowledge Graph (entities, relationships) + vec0 indexes for ANN search
- **Fact Dedup**: UNIQUE constraint on (agent_id, scope, key) — repeated mentions update content and bump mention_count

### 7. Wards — Domain-Scoped Delegatable Agents

A ward is both a persistent working directory **and** a delegatable specialist agent. The orchestrator can either spawn into an existing ward ("warm") or, when no ward matches the request, the planner builds a new one on demand ("cold"). Each ward carries its own doctrine, procedures, memory namespace, and optional per-ward LLM configuration.

**What lives in a ward**
- `AGENTS.md` — doctrine: scope, mandate, capabilities, out-of-scope rules, runtime hints (tech stack, build commands, conventions) — all in one file
- `config.yaml` — per-ward LLM override (`provider` / `model`; null inherits orchestrator)
- `memory-bank/ward.md` — curated rules (corrections, strategies, warnings)
- `memory-bank/core_docs.md` — function signatures across the ward's code
- `memory-bank/structure.md` — directory tree snapshot
- Project files — code, reports, datasets, whatever the ward produces

**Lifecycle**
- **Warm path** — intent analysis matches an existing ward → delegate directly
- **Cold path** — no match → planner constructs ward doctrine + procedures, then runs
- **Graduation gate** — a ward only becomes routable after passing telemetry thresholds (recent success rate, fact density)
- **Out-of-scope re-routing** — if a request lands in the wrong ward, the runtime re-routes to the right one rather than letting the ward stretch its mandate
- **Anti-fragmentation** — naming collision detection prevents `maritime-tracking` vs `maritime-vessel-tracking` style drift that would split recall

**Ward Curator (autonomous self-improvement)**
- **Telemetry (Phase A)** — per-ward success rate, fact growth, recall hit rate (rolling window)
- **Heuristic cleanup (Phase B)** — weekly cron consolidates stale facts, prunes archive, dedupes
- **LLM consolidation (Phase C)** — periodic LLM pass merges redundant rules, sharpens doctrine, surfaces conflicts

**Per-ward LLM config**
- Set `provider` / `model` in `config.yaml` to route that ward to a specific model — e.g. financial analysis on a smart model, scratch on a fast one
- Defaults to inherit from the Orchestrator setting

### 8. Observability Dashboard
Full execution visibility at `/mission-control` (`/logs` and `/dashboard` redirect there). Implementation: `apps/ui/src/features/mission-control/`.

**KPI strip (top):**
- Aggregate counters across the visible sessions

**Session List (left panel):**
- Filterable list of root sessions with status badges
- Agent count, duration, token usage per session
- Real-time updates for running sessions

**Session Detail (right panel):**
- Messages pane: chat narrative for the selected session
- Tools pane: per-agent tool calls with detail popover
- Subagent delegations grouped under their parent
- Errors highlighted

**Session controls:**
- Pause, resume, cancel running sessions
- Resume crashed sessions at the subagent level (smart resume)

### 9. Multi-Turn Session Management
Conversations persist across multiple turns within a session:

- **Session Continuity**: Multiple messages share the same session until `/new`
- **Context Preservation**: Full conversation history maintained per session
- **Session Reset**: `/new` command starts fresh session
- **Source Tracking**: Sessions tagged with origin (web, cli, api, etc.)

### 10. Scheduled Tasks
Cron-based scheduling for automated agent invocations. Define recurring tasks that run agents on a schedule. Backend uses `tokio-cron-scheduler` (6-field cron expressions); `kind:http` actions trigger arbitrary HTTP endpoints, `kind:agent` re-invokes the root agent with a prompt.

### 11. Per-Task LLM Routing
Different jobs deserve different models. Settings > Advanced exposes one card per LLM role; each card has a Provider dropdown (with **Inherit from Orchestrator**) and a Model text input.

| Slot | What it controls |
|------|------------------|
| **Orchestrator** | Root agent — the default everything else inherits from |
| **Distillation** | Post-session fact extraction |
| **Curator** | Ward consolidation cycles (Phase C) |
| **Intent Analysis** | Per-prompt routing classifier |
| **Sleep-time Pipeline** | Memory-cycle stages: synthesis, beliefs, abstraction, contradiction detection, pairwise verifier, handoff summarization |
| **Multimodal** | Vision analysis (`multimodal_analyze` tool) |

Resolution chain per call: **per-task override → orchestrator → provider default**. Untagged callers (chat, recall) stay on the orchestrator. This lets users run cheap models for nightly memory work, fast models for routing, and a smart model for actual reasoning — without touching the orchestrator.

### 12. Agent Pool — Steerable Running Subagents
Long-running subagents are addressable while alive: `wait_agent(execution_id)` blocks until completion and returns the result, `kill_agent(execution_id)` cancels a misbehaving one. The orchestrator can fan out work, do other things, then collect results — no synchronous-only delegation.

### 13. Observatory — 3D Knowledge Visualization
Real-time 3D rendering of the knowledge graph and hierarchy at `/observatory`. Apple-Vision aesthetic, hierarchy shells, ambient pulses on activity, RAG overlay during recall, entity-graph layer with typed edges. Built for inspecting the brain, not just admiring it.

## Technology Stack

| Layer | Technology |
|-------|------------|
| Frontend | React 19 + TypeScript + Vite |
| UI | Tailwind CSS v4 + Radix UI |
| Backend | Rust (Axum + tokio) |
| Database | SQLite (rusqlite + r2d2 pool, WAL mode) |
| API | HTTP REST + WebSocket |

## Roadmap

### Completed
1. **v0.1** — Core chat, providers, skills
2. **v0.2** — Persistent memory, SQLite conversations
3. **v0.3** — MCP integration, CLI improvements
4. **v0.4** — Operations Dashboard, Session Management
   - Real-time session monitoring
   - Multi-turn conversation support
   - Trigger source tracking
   - Subagent delegation visibility
5. **v0.5** — Responsive Architecture + Code Wards
   - Real streaming (no simulated delays)
   - SQLite WAL mode + r2d2 connection pool
   - Batch DB writes, RwLock caching
   - Parallel tool execution + output truncation
   - Gateway crate decomposition (13 crates)
   - Code Wards (persistent project directories)
   - Shell-first execution with dedicated write_file / edit_file tools
   - Session Tree (continuous conversation, subagent isolation)
   - Goal-oriented execution (scoring, stuck-detection, safety valve)
   - 300+ tests across all crates
6. **v0.6** — Memory Evolution
   - Embedding providers: local fastembed (default) + OpenAI-compatible
   - Hybrid search: FTS5 BM25 + vector cosine similarity
   - Session distillation: auto-extract facts from completed sessions
   - Smart recall: inject relevant facts at session start
   - Pre-compaction memory flush: save facts before context trim
   - Knowledge graph integration: entity/relationship extraction during distillation
   - Old standalone knowledge_graph tools removed (5 tools → unified `graph` action in memory tool)
   - 620+ tests across all crates

7. **v0.7** — Cognitive Memory
   - Bi-temporal facts (`valid_from` / `valid_to`, point-in-time recall)
   - Hierarchical memory (HiRAG / LeanRAG cluster builder + LCA-bounded recall)
   - Reflective synthesis: pattern abstraction, conflict resolution, pairwise entity verifier
   - Confidence-aware graph traversal + bulk decay
   - Procedures: learned runbooks become dispatchable `run_procedure` tool calls
   - 1000+ tests across all crates
8. **v0.8** — Ward Architecture
   - Ward-as-agent: wards become delegatable specialists with doctrine + procedures
   - Cold-path planner builds new wards on demand
   - Graduation gate, out-of-scope re-routing, anti-fragmentation guards
   - Ward curator: telemetry → heuristic cleanup → LLM consolidation
   - Per-ward LLM config (`config.yaml`)
   - Per-task LLM routing (Curator, Intent Analysis, Sleep-time, Distillation, Multimodal)
   - Agent pool: `wait_agent` / `kill_agent` for steerable subagents

### Planned
9. **v0.9** — Belief Network + Federation
   - Belief network: multi-fact beliefs with confidence propagation, contradiction graph, recall integration
   - Peer messaging between agents (intra-daemon `message_agent` + `list_agents`)
   - Federation transport (cross-daemon, role-name routing)
10. **v1.0** — Stable API, documentation, packaging
    - Context window management (auto-compaction, token counting)
    - Agent sandbox (process isolation)
    - Skill lifecycle (TTL, LRU, dependencies)

### Stretch Goals
- **Cross-Agent Gossip**: Agent A saves shared fact → all agents see it at next session start. Collective intelligence
- **Dream Mode**: During idle time, agent reviews its own memory, finds connections, generates new insights. Runs as cron job
- **Memory Diff**: Show user what the agent learned this session: "I learned 3 new facts: [...]". Transparency
- **Pluggable graph backend**: GraphTraversal trait → Neo4j when scale demands it
