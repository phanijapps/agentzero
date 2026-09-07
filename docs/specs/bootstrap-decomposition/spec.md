# Spec: Invoke bootstrap decomposition

- **Status:** Implementing
- **Branch:** `op_clean_crap`

## Objective

Decompose `gateway/gateway-execution/src/runner/invoke_bootstrap.rs` (2,623
lines; 2,001 production + 622 test). Three god-methods + the helper layer.

## The three god-methods

1. `finish_setup` — 320 lines. The phase-2 completion: history load,
   recall injection, executor construction, intent analysis wiring,
   stream handoff.
2. `create_executor` — 340 lines. Agent/provider resolution, builder chain,
   per-actor config, ward template injection.
3. `run_intent_analysis` — 256 lines. Intent classification pipeline
   orchestration (calls the already-clean `intent::analyze_intent`).

## Decomposition plan

- `finish_setup` → extract `load_history_with_recall(...)`,
  `wire_intent_outcome(...)`, `build_stream_inputs(...)`. Orchestrator ≤100.
- `create_executor` → extract `resolve_agent_and_provider(...)`,
  `configure_actor_execution(...)`, `inject_ward_context(...)`. Orchestrator ≤100.
- `run_intent_analysis` → extract `build_intent_llm_client(...)`,
  `apply_intent_overrides(...)`. Orchestrator ≤120.

The helpers outside the impl (363-608) are already well-shaped — leave them.

## Non-goals

No behavior change. No file splitting. The InvokeBootstrap struct is
already { ctx } from Wave 1 — this wave decomposes the methods only.
