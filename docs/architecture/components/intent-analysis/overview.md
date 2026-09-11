# Intent Analysis — Component Overview

## What It Is

Intent analysis is **pre-execution middleware** for root-agent sessions. It
runs before the first LLM call, decides the shape of the task, and injects a
`## Task Analysis` section into the system prompt. The result is:

- **Injected into the agent's system prompt** via `format_intent_injection()` so the agent follows ward/skill/strategy recommendations
- **Emitted as WebSocket events** for the UI to display
- **Persisted to execution logs** for session replay

Source: `gateway/gateway-execution/src/middleware/intent/` (agent.rs,
contract.rs, inject.rs, prompt.rs, router.rs).

## When It Runs

- Only for root agent invocations
- Runs in the invoke bootstrap, before the executor is built

## What It Does (current design — agent-driven, post-2026-09 rewrite)

1. **Route** (`router.rs`) — greetings and non-task messages bypass the LLM
   entirely; a deterministic procedure match pins `run_procedure` directly;
   everything else goes to the intent agent.
2. **Intent agent** (`agent.rs`) — a small tool-carrying agent whose model
   searches indexed resources itself via `MemorySearchTool` (skills, agents,
   wards, procedures), reasons over the request, and writes its conclusion.
3. **JSON contract** (`contract.rs`) — the agent's text output is parsed with
   `serde_json::from_str`. Fields: `primary_intent`, `hidden_intents`,
   `solution_path` (high-level steps that seed the planner), `complexity`
   (S/M/L/XL — sets iteration budget), `recommended_skills`/`agents`/
   `procedures`/`capabilities`, `ward_recommendation`, `execution_strategy`.
   No `response_format` is used — structured-output modes break on Ollama.
4. **Inject** (`inject.rs`) — renders the `## Task Analysis` prompt section;
   pinned procedures get a "call run_procedure directly" instruction.
5. **Emit + persist** — `IntentAnalysisStarted`/`Complete` events for the UI;
   the analysis logs to `execution_logs` for replay.

## What It Does NOT Do

- Does NOT pre-fetch resources into the prompt (the agent searches)
- Does NOT auto-load skills or auto-delegate
- Does NOT run for subagents
- Does NOT block execution on failure (all errors are non-fatal)

## Key Design Decisions

- **Plain-JSON parse over response_format**: `json_schema` response formats
  produce empty responses on Ollama; the agent writes JSON as text and serde
  parses it.
- **Trivial bypass**: one-word greetings skip the LLM call entirely.
- **Procedure pinning**: a deterministic name/trigger match short-circuits
  planning — the model is told to run the procedure as-is.

## Related Docs

- [types.md](types.md) — Rust + TS types, field mapping (verify against
  `contract.rs`; field set evolved with the rewrite)
- [error-handling.md](error-handling.md) — degradation hierarchy
- [files.md](files.md) — file reference
- Contract source of truth: `gateway/gateway-execution/src/middleware/intent/contract.rs`
