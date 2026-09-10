# Spec: Delegation spawn decomposition

- **Status:** Implementing
- **Branch:** `op_clean_crap`

## Objective

Decompose `gateway/gateway-execution/src/delegation/spawn.rs` (2,655 lines;
1,920 production + 735 test) the same way as executor.rs and distill():
find the missing types, extract the phases, shrink the god-functions.

## The two god-functions

1. `spawn_delegated_agent(...)` — **734 lines**, 28 positional parameters
   (the last context-restatement that predates ExecCtx — this function IS
   an ExecCtx consumer but takes everything positionally)
2. `spawn_execution_task(ctx: SpawnContext)` — **366 lines** — the child's
   stream loop: engine construction, event processing, checkpoint,
   success/failure handling

## Decomposition plan

### spawn_delegated_agent (734 → ~120 orchestrator)

Natural phases (read the body, find the boundaries):
- `resolve_child_config(...)` — agent loading, actor kind, delegation mode,
  dynamic skill resolution, capability assignment validation
- `create_child_session(...)` — session creation, execution creation,
  child session linking, ward binding
- `prime_child_context(...)` — ward context injection, unified recall priming,
  handoff context
- `build_child_executor(...)` — the ExecutorBuilder chain with all the
  conditional wiring (mcps, skills, fact store, kg, steering, procedure)
- `launch_child(...)` — handle registration, engine construction (already
  routes through `build_execution_engine`), spawn the task

The 28 positional parameters become `&ExecCtx` + `&DelegationRequest` +
the few per-child values (child_agent_id, parent info, permit). This is
the last pre-ExecCtx signature in the codebase.

### spawn_execution_task (366 → ~150 orchestrator)

Natural phases:
- `drive_child_stream(...)` — the event loop (already largely delegates to
  `process_stream_event`; the inline accumulation is what's left)
- `finalize_child(...)` — checkpoint, flush, callback, completion handling
  (already extracted as `handle_execution_success` / `handle_execution_failure` —
  verify and keep)

### What stays

`handle_execution_success`, `handle_execution_failure`,
`handle_early_spawn_failure`, `build_crash_report` — these are already
well-typed with parameter structs (the pattern is right). The capability
resolution helpers (787-955) are fine as-is.

## Acceptance Criteria

- [ ] AC1: `spawn_delegated_agent` ≤ 150 lines, takes `&ExecCtx` + request
- [ ] AC2: `spawn_execution_task` ≤ 200 lines
- [ ] AC3: The 28 positional parameters → ExecCtx + per-child struct
- [ ] AC4: All 600 tests + delegation flow test + golden traces green
- [ ] AC5: clippy -D warnings, fmt clean

## Non-goals

No behavior change. No file splitting — everything stays in spawn.rs
unless it exceeds 1,200 after decomposition (then split into spawn.rs +
spawn_phases.rs).
