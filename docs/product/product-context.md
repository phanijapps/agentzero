# z-Bot — Product Context

## Why z-Bot Exists

z-Bot is a **goal-oriented AI agent** that lives on your desktop. It is not a chatbot — it is an autonomous execution engine. Give it a goal, and it analyzes intent, plans an approach, delegates to specialist agents, executes tools, learns from results, and persists knowledge for future sessions.

The core insight: most AI tools are designed around **control** (you tell it exactly what to do, step by step). z-Bot is designed around **goals** (you tell it what you want to achieve, and it figures out how). This enables long-running autonomous sessions where agents iterate, recover from failures, and coordinate with each other — without constant human steering.

## Problems It Solves

1. **AI tools require constant steering** — Most assistants need you to drive every step. z-Bot's intent analysis, planning, and delegation mean you set a goal and walk away. The agent figures out the approach, delegates to specialists, and iterates until the goal is achieved.

2. **No cross-session learning** — Most AI assistants forget everything between sessions. z-Bot automatically distills session transcripts into structured facts (corrections, strategies, domain knowledge), then recalls relevant facts at the start of new sessions via hybrid semantic + keyword search. The agent learns without manual effort.

3. **Single-agent limitations** — Complex tasks need different skills (planning, coding, research, documentation). z-Bot's multi-agent delegation lets a root orchestrator break work into pieces and assign them to specialist subagents, each with their own tools, context, and instructions.

4. **Provider lock-in** — Most platforms work with one LLM provider. z-Bot is provider-agnostic: OpenAI, Anthropic, DeepSeek, Groq, Ollama, or **any OpenAI-compatible API**. Configure different models for orchestration, distillation, and multimodal analysis independently.

5. **Data sovereignty** — Cloud AI platforms store your conversations, code, and context on their servers. z-Bot keeps everything on your machine in plain files and SQLite. Embeddings run locally via ONNX (zero API cost).

6. **No persistent project context** — Session-based AI assistants start from scratch every time. z-Bot's Code Wards give agents persistent project directories they create, name, and navigate autonomously. Code survives across sessions.

## Core Principles

1. **Goal-Oriented** — The agent's job is to achieve your goal, not to wait for instructions. Intent analysis, planning, and autonomous execution are the defaults.
2. **Self-Learning** — Every session teaches the agent something. Distillation extracts facts; recall injects them. Corrections are surfaced first so the agent never repeats the same mistake.
3. **Local-First** — Your data stays on your machine. No cloud lock-in. Embeddings run locally.
4. **Provider-Agnostic** — Use any LLM: OpenAI, Anthropic, DeepSeek, Ollama, self-hosted. Mix providers across orchestration, distillation, and vision.
5. **Extensible** — Skills, MCP servers, and Bridge Workers let you add any capability without touching core code.

## Target Users

| User | Use Case |
|------|----------|
| **Developers** | Autonomous coding assistants that plan, implement, test, and iterate on their own |
| **Power Users** | Long-running research, analysis, and content creation workflows |
| **Teams** | Managing multiple specialist agents with different capabilities and project contexts |
| **Privacy-Conscious** | Running AI locally without sending data to third parties |

## How It Should Work

1. **Set a goal** — Type what you want to achieve (not step-by-step instructions)
2. **Agent analyzes intent** — Determines complexity, selects specialist agents, identifies relevant skills and wards
3. **Agent plans** — For non-trivial tasks, creates a structured plan before executing
4. **Agent delegates** — Root orchestrator dispatches tasks to specialist subagents (planner, coder, researcher, tutor)
5. **Subagents execute** — Each subagent works autonomously with tools (shell, file editing, web search, memory)
6. **Agents iterate** — If tests fail, the agent fixes and retries. Complexity budgets prevent infinite loops.
7. **Results collected** — Subagent results flow back to root via callbacks. Root can delegate further or respond.
8. **Memory distilled** — After session completes, facts, entities, and relationships are extracted into persistent memory
9. **Next session starts smarter** — Relevant corrections, strategies, and domain knowledge are recalled automatically

## Token Usage Warning

z-Bot is **token-intensive by design**. A single user request can trigger:
- Intent analysis (1 LLM call)
- Planning (1 LLM call per subagent)
- Subagent execution (5-50+ LLM calls per subagent, depending on task complexity)
- Post-session distillation (1-2 LLM calls)
- Memory recall with graph expansion

A complex coding task with 3 subagents can easily consume 100+ LLM calls and 200k+ tokens in a single session. This is the cost of autonomy — the agent is doing the work that a human developer would otherwise do manually. Monitor your provider usage dashboard and configure rate limits in Settings.

## Differentiators

| Feature | z-Bot | Cloud AI Platforms |
|---------|-------|-------------------|
| Execution Model | Goal-oriented, autonomous | Request-response, human-steered |
| Multi-Agent | Built-in orchestration + delegation | Rare / manual |
| Memory | Auto-distillation + semantic recall + knowledge graph | None / session-scoped |
| Data Location | Local machine | Cloud servers |
| Provider Lock-in | None — any OpenAI-compatible API | Usually locked |
| Multimodal | Configurable vision model (any provider) | Platform-specific |
| Offline Capable | Yes (with Ollama) | No |
| Cost | API costs only (embeddings free via local ONNX) | Subscription + API |
| Customization | Agents, skills, MCP servers, bridge workers | Limited |
| Persistent Code | Code Wards (survive across sessions) | Session-scoped |
| Observability | Timeline tree with full tool call visibility | Limited logs |
