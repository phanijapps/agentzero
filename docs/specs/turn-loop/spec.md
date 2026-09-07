# Spec: Turn loop decomposition (Wave 4)

- **Status:** Implementing
- **Branch:** `op_clean_crap`
- **Shape:** refactor (behavior-preserving), gated by golden traces

## Objective

Decompose `runtime/agent-runtime/src/rig_adapter/engine.rs` (1,659 lines) by
data modeling: extract the missing domain types so the loop reads as
orchestration, not parsing. The `select!` concurrency skeleton (stop/steer/
heartbeat/policy-events) is battle-tested and STAYS — this wave extracts the
inline parsing/mapping bodies that bloat the loop, not the loop itself.

## Non-goals (explicit)

- No big-bang rewrite of the run loop.
- No change to `execution_stream.rs` / `stream_event_processor.rs` — those
  collapse in a later pass once the engine internals are typed.
- No behavior change: golden traces + 600 tests + parity suites are the gate.

## Extractions

1. **`rig_adapter/turn_signal.rs`** — the loop's terminal conditions as ONE
   enum: `enum TurnSignal { Continue, Stop, DelegationYield, Responded,
   TurnLimit }`. The scattered `stopped_for_delegation` / `responded` /
   `limit_reached` booleans become one match at the loop bottom.
2. **`rig_adapter/turn_events.rs`** — the `MultiTurnStreamItem` → `StreamEvent`
   mapping arms (Text/ToolCall/Reasoning deltas, ToolResult outcomes,
   CompletionCall usage) as pure functions `map_assistant_item(...)`,
   `map_user_item(...)`, `map_completion_call(...)`. The `match item` body
   in `run_inner` becomes dispatch to these.
3. **`rig_adapter/engine.rs` shrinks to**: construction, run/run_inner
   orchestration (select! skeleton + signal match), checkpoint emission,
   cleanup. Target under 700 lines.

## Acceptance Criteria

- [ ] AC1: `TurnSignal` is the sole terminal-state carrier; no bare
      `stopped_for_delegation`/`responded` booleans escape the loop body.
- [ ] AC2: The item-mapping arms are pure functions, unit-testable without
      a live engine.
- [ ] AC3: Golden traces replay byte-identical (3 fixtures).
- [ ] AC4: agent-runtime suite (was 449+ new hook tests), gateway-execution
      600 lib tests, parity suites, clippy -D warnings, fmt — all green.

## Gate

Any golden-trace diff blocks the wave. Regenerate is NOT allowed except via
an explicit human decision recorded in parity-matrix.
