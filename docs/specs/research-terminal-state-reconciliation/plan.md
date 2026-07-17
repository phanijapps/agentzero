# Plan: Research Terminal State Reconciliation

- **Spec:** [`spec.md`](spec.md)
- **Status:** Shipped

## Approach

Make the terminal transition authoritative in the execution-state service,
then project its changed snapshot into the existing plan surface before the
completion event. Prefer that same session snapshot when building continuation
context. Finally, close the response timing gap by draining queued durable
messages before terminal events and rendering the already-present completion
result in Research.

## Constraints

- Reuse `session_plans`, `GatewayEvent::SurfaceUpdated`, and the existing
  Research reducer; do not change public message shapes or add storage.
- Preserve failed step state and leave non-success terminal paths unchanged.

## Construction tests

**Integration tests:** terminal state persistence plus emitted surface ordering.

**Manual verification:** run a delegated Research session with an active plan,
wait for completion, and confirm all non-failed checklist rows and the final
answer are visible without a reload.

## Design (LLD)

### Design decisions

`StateService` terminalizes its accepted session plan only after
`try_complete_session` succeeds. The lifecycle emits the resulting existing
native surface before `agent_completed`; this makes terminal UI state
independent of model tool-call discipline. Traces to: AC 1, AC 2, AC 5.

### State & control flow

`respond` writes are drained before lifecycle completion. On the client,
`agent_completed.result` is applied as root turn content before closing the
Research or Quick Chat turn, so it remains a fallback if the preceding
`turn_complete` was lost. Traces to: AC 4, AC 5.

### Failure, edge cases & resilience

No plan, an already-terminal plan, failed/cancelled execution, and a missing
completion result remain no-op paths. A persistence failure is logged and does
not block the existing completion event. Traces to: AC 1, AC 2.

### Dependencies & integration

The correction links execution-state persistence, gateway events, batch
writing, and the existing Research reducer. It does not add an external
contract. Traces to: AC 2, AC 4, AC 5.

## Tasks

### T1: Persist a successful terminal plan snapshot and publish it

**Depends on:** none

**Touches:** `services/execution-state/src/service.rs, services/execution-state/src/repository.rs, gateway/gateway-execution/src/lifecycle.rs, gateway/gateway-execution/src/invoke/stream_event_processor.rs`

**Tests:**
- A successful session terminalizes pending/in-progress steps while preserving
  completed/failed states (AC 1).
- A changed terminal plan produces `SurfaceUpdated` before `AgentCompleted`
  (AC 2).

**Approach:**
- Add an idempotent trusted state-service operation that transforms only the
  current accepted session plan after the session is actually completed.
- Project its returned snapshot through the existing plan-surface builder.

**Done when:** focused Rust tests prove both the stored status transition and
event order.

### T2: Use the current session plan for continuation context

**Depends on:** T1

**Touches:** `gateway/gateway-execution/src/runner/core.rs`

**Tests:**
- A continuation with both a session plan and a newer unrelated ward plan
  renders only the session plan (AC 3).

**Approach:**
- Load the accepted plan from state and render it ahead of the legacy ward-plan
  fallback in continuation context.

**Done when:** continuation tests prove a ward's unrelated latest plan cannot
replace the active session plan.

### T3: Make terminal responses durable and visible

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/invoke/batch_writer.rs, gateway/gateway-execution/src/runner/execution_stream.rs, gateway/gateway-execution/src/runner/core.rs, gateway/gateway-execution/src/delegation/spawn.rs, apps/ui/src/features/research-v2/event-map.ts, apps/ui/src/features/research-v2/reducer.ts, apps/ui/src/features/chat-v2/event-map.ts, apps/ui/src/features/chat-v2/reducer.ts`

**Tests:**
- A batch-writer drain persists queued messages before its acknowledgement
  (AC 5).
- A Research or root Quick Chat `agent_completed.result` visibly becomes the
  completed turn's answer (AC 4).

**Approach:**
- Add a queued-write drain acknowledgement before lifecycle completion.
- Carry the completion result into the reducer and apply it before closing the
  turn.

**Done when:** focused Rust and Vitest regression tests pass without relying on
the earlier `respond` event.

### T4: Preserve continuation responses and deduplicate terminal delivery

**Depends on:** T3

**Touches:** `gateway/gateway-execution/src/invoke/response_accumulator.rs, gateway/gateway-execution/src/invoke/mod.rs, gateway/gateway-execution/src/runner/execution_stream.rs, gateway/gateway-execution/src/runner/core.rs, apps/ui/src/features/research-v2/turns.ts, apps/ui/src/features/research-v2/reducer.ts`

**Tests:**
- TDD: a tool-only `respond` turn resolves to the response argument for either
  root execution path.
- TDD: an earlier plain progress message followed by a later `respond` payload
  rehydrates to the terminal response.
- TDD: duplicate root completion delivery returns the existing completed state.

**Approach:**
- Share the existing normal-execution content resolver with the continuation
  runner instead of allowing its tool-only response row to become a placeholder.
- Select the latest assistant-answer candidate during snapshot reconstruction.
- Make duplicate terminal delivery an identity-preserving reducer no-op.

**Done when:** continuation output survives both live completion and a snapshot
reload, while repeated terminal events do not create another Research render.

## Rollout

Ship as a reversible code-only correction. Existing incomplete historical plans
remain unchanged; new successful completions publish the reconciled terminal
surface.

## Risks

- A terminalizer must run only after all executions finish, otherwise it could
  prematurely complete active delegated work.
- The drain must acknowledge only writes already queued by the stream, without
  introducing unbounded waits or blocking normal token streaming.

## Changelog

- 2026-07-16: initial plan.
- 2026-07-16: implemented and verified.
- 2026-07-17: reopened T4 after a delegated Research continuation persisted its
  final `respond` payload as a placeholder and snapshot recovery overwrote the
  live answer.
- 2026-07-17: completed T4 with a shared persisted-answer resolver,
  chronological snapshot selection, terminal-event idempotency, and a
  full-mode continuation reload regression test.
