# Execution Loop — Component Overview

## What It Is

The execution loop is the **end-to-end pipeline** that processes a user message — from the moment it arrives via WebSocket through intent analysis, LLM execution, tool calls, delegation, and final response delivery. It is the central nervous system of AgentZero.

## Two Modes of Operation

### New Session (First Message)
Full pipeline: session creation → memory recall → intent analysis → prompt compilation → LLM loop → delegation → continuation → response → distillation.

### Continuation (Subsequent Message in Existing Session)
Abbreviated pipeline: session reuse → history reload → intent skip → LLM loop → response. No intent analysis re-run; full conversation history is loaded from the database.

## Key Design Decisions

- **Session = conversation container**: A `session` groups all messages and executions for one conversation. The root execution is reused across continuation turns (same `execution_id`), enabling the `has_intent_log()` gate.
- **Intent analysis is a one-shot gate**: Checked via `execution_logs WHERE session_id = execution_id AND category = 'intent'`. When skipped, an `IntentAnalysisSkipped` event is emitted so the UI can skip the "Analyzing intent" phase.
- **Delegation is async-sequential per session**: A background handler processes delegation requests through a per-session queue with a global concurrency semaphore (default: 2 concurrent delegations).
- **Continuation is event-driven**: After all delegations complete, a `SessionContinuationReady` event triggers the root agent to resume with delegation results injected into history.
- **Distillation is post-session**: After the session completes (all executions terminal), an LLM extracts facts, entities, relationships, and episode assessments into the memory system.
- **Streaming-first**: All LLM calls stream tokens. Events are emitted in real-time via the event bus and routed to WebSocket subscribers.

## Pipeline Stages (New Session)

```
UI Message → WebSocket Handler → Runtime Service → Runner
  ├── 1. Session & Execution Creation (DB: sessions, agent_executions)
  ├── 2. Agent & History Loading (DB: messages, vault YAML)
  ├── 3. Memory Recall (DB: memory_facts, knowledge_graph, recall_log)
  ├── 4. Intent Analysis (LLM call → DB: execution_logs)
  ├── 5. Prompt Compilation (templates + intent injection + skills)
  ├── 6. Executor Build (LLM client + tools + middleware)
  ├── 7. Execution Loop (LLM ↔ tools, streaming tokens)
  │     ├── Tool calls → shell, file ops, memory, ward
  │     ├── Delegation → spawn subagent (async)
  │     └── Respond → final message via hook
  ├── 8. Delegation Handler (per-session queue, semaphore)
  ├── 9. Continuation (root resumes with delegation results)
  ├── 10. Completion (DB: sessions, agent_executions, execution_logs)
  └── 11. Distillation (LLM → DB: memory_facts, knowledge_graph, session_episodes)
```

## Pipeline Stages (Continuation)

```
UI Message (with session_id) → WebSocket Handler → Runtime Service → Runner
  ├── 1. Session Reuse (get_or_create_session → reuse root execution)
  ├── 2. History Reload (DB: messages — full conversation, up to 200)
  ├── 3. Memory Recall (same as new session)
  ├── 4. Intent Analysis SKIPPED (emit IntentAnalysisSkipped event)
  ├── 5–11. Same as new session
  └── UI shows "Executing" phase directly (no "Analyzing intent")
```

## Related Docs

- [data-flow.md](./data-flow.md) — Complete call sequence with file paths, line numbers, DB operations, and events
- [files.md](./files.md) — Every file involved with function-level references
- [../intent-analysis/overview.md](../intent-analysis/overview.md) — Intent analysis middleware details
- [../memory-layer/overview.md](../memory-layer/overview.md) — Memory recall and distillation details
