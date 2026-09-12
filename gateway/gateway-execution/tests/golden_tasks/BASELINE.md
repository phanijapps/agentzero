# Task-Level Golden Runs — Baseline & Contract

End-to-end scripted sessions asserting agent-loop SIDE EFFECTS. Distinct
from `golden_trace_tests` (event-sequence parity within one turn stream)
and `golden_recall` (memory ranking floors): this harness drives the full
ExecutionRunner through multi-agent flows and asserts observable state.

Run: `cargo test -p gateway-execution --features test-stubs --test golden_tasks -- --ignored --nocapture`

## What each scenario guards

| Scenario | Guards | The live bug it would have caught |
|---|---|---|
| `ward_then_plan` | root ward binding; **planner child session ward inheritance**; **planner prompt roster manifest** (`## Available Agents` with the roster — asserted against the captured LLM request body); zero `lookup_capabilities` calls; ward scaffolding on disk | sess-70d057a3 (planner spawned ward-less 9ms after ward creation → `planner_template_unavailable`); sess-fd588249 (12 `lookup_capabilities` calls hunting a roster the catalog structurally couldn't contain) |
| `procedure_contract` | seeded procedure with declared parameters executes via `run_procedure` with supplied args; `{args.X}` interpolation end-to-end (shell stdout in `all_steps`); success/failure counters increment | sess-05ba0fd4 (procedure recalled without its Parameters contract → bare call → arg-validation error → turns of manual recovery) |
| `memory_persistence` | `memory_write` fact durable in the store with the written shape (asserted via the `/api/memory` list path — the harness wires no embedding client, so semantic recall degrades by design) | the memory-into-the-void class (pre-consolidation KV writes nothing read) |
| `simple_fast_path` | trivial prompt → direct respond; no delegation events; no ward directories created | regression guard for the fast-path routing (graph-path creep) |
| `parallel_join` | two `parallel: true` delegations back-to-back: **both accepted** (second never hits the per-session claim rejection), both child sessions spawn with correct agents and parent linkage, **root resumes only after BOTH children complete** (continuation-watcher join, asserted by event-arrival order), final respond reached, no lookup_capabilities | silent per-session claim regression (parallel delegations suddenly queued/blocked); join/resume semantics drift |

**Parallel semantics note**: the harness runs `max_parallel_agents: 1`, so
the second child queues at the **dispatcher semaphore** (then runs) — never
at the per-session delegation claim. A higher semaphore yields true
concurrency; both orderings satisfy this scenario, which pins acceptance,
completion, and join — not interleaving.

**Layered coverage map for `parallel`**: tool layer —
`delegate.rs::parallel_delegates_are_not_blocked_by_claim` (unit); task
layer — `golden_task_parallel_join` (accept + join + resume); the flag's
plumbing (dispatcher semaphore routing) — covered transitively by the
scenario's acceptance assertions. Earlier research flagged the field
unread (spawn.rs-only grep) — false positive; dispatcher reads it.

## Harness notes

- **ScriptedProvider**: raw-TCP SSE server (same technique as the runner's
  `test_support`). Each accepted connection is one completion request;
  caller identity routes by unique instruction markers seeded per agent
  (`PLANNERMARK` / `BUILDERMARK`), unmatched requests run the `__root__`
  script. Request bodies are captured for prompt-level assertions.
- **Intent requests** (`"You are an intent analyzer"` system prompt) are
  served a canned simple-intent JSON — they never consume scripted turns.
- **Ward archetype registry** must be seeded in the harness
  (`gateway_services::seed_default_ward_archetypes`) — production does this
  at AppState bootstrap; a direct-runner harness skips AppState. First
  ward-create otherwise fails with `template_create_failed`.
- Stores are the engram in-memory sidecar adapters (same construction as
  the conformance suite) on a fresh tempdir per scenario.

## Known gaps (documented, not hidden)

- **Distillation is not asserted**: the harness wires `distiller: None`.
  A distill-on-completion scenario needs the full distiller construction
  (LLM transcript pass) — deferred until a stub-distiller seam exists.
- **Stop-midstream (task level)**: cancellation mid-delegation with clean
  child statuses is not covered — the stuck-`running` executions bug class
  remains asserted only by production observation.
- **Semantic recall** in `memory_persistence` is degraded (no embedding
  client); the ranking floors live in `gateway-memory/tests/golden_recall`.

## Adding a scenario

1. Script the turns (per-agent `Vec<Turn>`), keyed by a new agent marker or
   `__root__`.
2. `build_harness(provider)` → `run_to_completion(prompt)`.
3. Assert observable state: session rows (`state.get_session`),
   messages tape (`get_session_messages`), captured request bodies
   (`harness.bodies`), store contents, event-bus events.
4. Map it to the live bug it guards in the table above.

## Distiller seam (added)

`gateway_execution::distill::Distill` trait extracts the runner's single
distillation operation; the concrete `SessionDistiller` implements it by
delegation. The harness now wires a recording `DistillerStub` —
`memory_persistence` asserts the root completion dispatches a distill call
`(session_id, "root")`. Captured behavior: root completion fires exactly one
root distill; per-wave subagent distills (spawn.rs) fire per child completion
when children run (visible in ward_then_plan scenario logs).
