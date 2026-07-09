# Components Index

Quick reference for all system components. Check this before planning changes.

## Agent Execution

| Component | Location | Description |
|-----------|----------|-------------|
| Execution Loop | [execution-loop/overview.md](execution-loop/overview.md) | End-to-end pipeline: UI message → session → recall → intent → prompt → LLM loop → delegation → continuation → response → distillation. Covers new sessions and continuations. |
| — Data Flow | [execution-loop/data-flow.md](execution-loop/data-flow.md) | Complete call sequence with file paths, line numbers, DB operations, events, and prompt compilation |
| — Files | [execution-loop/files.md](execution-loop/files.md) | Every file across UI, gateway, runtime, services with function references |
| Subagent Capability Policy | [subagent-capability-policy/overview.md](subagent-capability-policy/overview.md) | Runtime actor/tool boundary for root, delegated executors, delegated reviewers, and ward agents. Documents reviewer-agent seeding and why ward-as-agent remains full-tool. |
| Agent Handoff Notes | [agent-handoff-notes/overview.md](agent-handoff-notes/overview.md) | Current-session agent roster and one-way handoff notes over existing steering. Not Pattern 4 peer messaging, persistence, or federation. |
| Intent Analysis | [intent-analysis/overview.md](intent-analysis/overview.md) | Pre-execution middleware: indexes resources, semantic search, LLM analysis, emits events. Root agent only. |
| Ward Scaffolding | [ward-scaffolding/overview.md](ward-scaffolding/overview.md) | Post-execution: skill-driven directory scaffolding, AGENTS.md generation, core module indexing via language configs. |
| — Data Flow | [intent-analysis/data-flow.md](intent-analysis/data-flow.md) | Live execution pipeline, session replay, WS event routing |
| — Types | [intent-analysis/types.md](intent-analysis/types.md) | Rust + TypeScript types, field mapping, log format |
| — Error Handling | [intent-analysis/error-handling.md](intent-analysis/error-handling.md) | Truncation repair, fallback events, degradation hierarchy |
| — Files | [intent-analysis/files.md](intent-analysis/files.md) | Every file with line numbers |

## Chat Experience

| Component | Location | Description |
|-----------|----------|-------------|
| Chat v2 | [chat-v2/overview.md](chat-v2/overview.md) | `/chat` page (live route; `/chat-v2` is a legacy redirect). Reserved session via `settings.chat` slot, `mode=fast`. Two-row status pill, artifact strip + slide-out, Clear button. |
| — Learnings | [chat-v2/learnings.md](chat-v2/learnings.md) | Hard-earned rules from the build — **apply these to any new UI plan** (server-owned identity, deterministic pill, wire-format field drift, StrictMode-safe bootstrap, etc.). |
| — Backlog | [chat-v2/backlog.md](chat-v2/backlog.md) | Pending: artifact auto-registration, context compaction, silent-crash surfacing, multi-tab sync, history pagination, E2E mock-LLM harness. |
| Research v2 | [research-v2/overview.md](research-v2/overview.md) | `/research` page (live route; `/research-v2` is a legacy redirect). Multi-turn research surface — replaced the old `/` MissionControl page; legacy MissionControl still reachable via `/mission-control`. One session per user prompt, drawer of past sessions, nested subagent cards with Request/Response + inline LiveTicker, live artifacts strip, clickable ward chip. Dual WS subscription (conv_id + session_id) with reconnect recovery and event-driven reconcile. |
| — Learnings | [research-v2/learnings.md](research-v2/learnings.md) | Hard rules learned during R14a–R14j (respond lives in toolCalls, delegation events carry no conv_id, session-scope filters child events, subscription ack races early events, ping-timeout reconnect loses invoke_accepted, LogSession.conversation_id is the sess-*, etc.). |
| — Backlog | [research-v2/backlog.md](research-v2/backlog.md) | Pending: LiveTicker UX (too brief in fast sessions), memory_facts_index defect, E2E harness, title fallback, delete cascade on child rows, retire old `/` page. |

## LLM Client

| Component | Location | Description |
|-----------|----------|-------------|
| LLM Client | [llm-client/overview.md](llm-client/overview.md) | Text & multimodal content pipeline: Part types (Text, Image, File), OpenAI-compatible content encoding, base64 flush persistence, `multimodal_analyze` tool, eagle-eye skill. Capability-aware encoding for OpenAI-compatible providers. |
| — Data Flow | [llm-client/data-flow.md](llm-client/data-flow.md) | Message lifecycle (text & multimodal), base64 flush, rehydration, tool flow, config injection |
| — Types | [llm-client/types.md](llm-client/types.md) | Rust + TypeScript types, Part/ContentSource/ImageDetail, ChatMessage serde, wire format mapping |
| — Error Handling | [llm-client/error-handling.md](llm-client/error-handling.md) | 8 error points: capability rejection, missing config, file not found, decode failures, API errors, backward compat |
| — Files | [llm-client/files.md](llm-client/files.md) | Every file across agent-primitives, agent-runtime, agent-tools, gateway, UI |

## Memory & Intelligence

| Component | Location | Description |
|-----------|----------|-------------|
| Memory Layer | [memory-layer/overview.md](memory-layer/overview.md) | The brain: facts, embeddings, knowledge graph, recall, distillation, ward knowledge sync. Six cooperating layers after Phases 1–6. |
| — Cognitive Layers | [memory-layer/cognitive-layers.md](memory-layer/cognitive-layers.md) | Working memory (Phase 2), Ward wiki (Phase 3), Procedural memory (Phase 4), Intelligent micro-recall (Phase 5). How each layer works and how they compose. |
| — Knowledge Graph | [memory-layer/knowledge-graph.md](memory-layer/knowledge-graph.md) | Phase 6 architecture: episodes, ward artifact indexer, expanded ontology (13 entity types / 27 relationship types), entity resolver, epistemic classes, MAGMA multi-view queries, real-time tool extraction. |
| — Data Model | [memory-layer/data-model.md](memory-layer/data-model.md) | Every table, every column, schema version history (v1 → v21), lifecycle events, query patterns. |
| — Backlog | [memory-layer/backlog.md](memory-layer/backlog.md) | Planned: policies UI, graph query tool, pruning, cross-ward synthesis, dashboard |

## Adding New Components

When adding a new component to `docs/architecture/components/`:
1. Create a subdirectory: `components/{component-name}/`
2. Add an `overview.md` with purpose, when it runs, design principles
3. Add additional docs as needed (data-flow, types, error-handling, files)
4. Add a row to the table above
