# Intent Analysis — Component Overview

## What It Is

Intent analysis is a **pre-execution middleware** that runs automatically before the root agent's first LLM call. It analyzes the user's message to determine intent, recommend resources, and plan execution strategy. The result is:
- **Injected into the agent's system prompt** via `format_intent_injection()` so the agent follows ward/skill/strategy recommendations
- **Emitted as WebSocket events** for the UI to display
- **Persisted to execution logs** for session replay

## When It Runs

- Only for root agent invocations (`is_root = true`)
- Only when a fact store is available (memory_repo + embedding_client configured)
- **Only on the first turn of a session** — skipped on continuation turns via `has_intent_log()` gate
- Runs inside `create_executor()` in `runner.rs`, before the executor is built

## What It Does

1. **Index Resources** — Upserts skills, agents, and wards into `memory_facts` (idempotent, no LLM call)
2. **Semantic Search** — Finds top-N relevant resources for the user's message using local embeddings
3. **LLM Analysis** — Sends user message + relevant resources to LLM, gets structured JSON back
4. **Emit Events** — Publishes `IntentAnalysisStarted` and `IntentAnalysisComplete` WebSocket events
5. **Inject into Agent** — Appends `## Intent Analysis` section to `agent.instructions` via `format_intent_injection()`
6. **Persist** — Logs full analysis to `execution_logs` with `LogCategory::Intent` for session replay

## What It Does NOT Do

- Does NOT auto-load skills or auto-delegate to agents
- Does NOT run for subagents or continuation turns
- Does NOT block execution on failure (all errors are non-fatal)

## Key Design Decisions

- **Session-aware gate**: Uses `has_intent_log(execution_id)` to check if intent was already analyzed for this session. Prevents redundant LLM calls on follow-up messages.
- **OnSessionReady callback**: The runner accepts an optional async callback that fires after session creation but before events emit. The WS handler uses this to subscribe the client before `IntentAnalysisStarted` fires, fixing a race condition where new sessions missed early events.
- **Lean prompt**: The LLM prompt requests only essential fields. `rewritten_prompt`, `structure` map, and `mermaid` diagram were removed to reduce token usage and parse failures. Approach simplified to `simple | graph` (no `tracked`).
- **No JSON repair**: Truncated JSON repair was removed. On parse failure, a clean fallback event is emitted instead.

## Related Docs

- [data-flow.md](./data-flow.md) — Complete event and data pipeline
- [types.md](./types.md) — All Rust and TypeScript types
- [error-handling.md](./error-handling.md) — Fallbacks and degradation
- [files.md](./files.md) — Every file involved with line numbers
