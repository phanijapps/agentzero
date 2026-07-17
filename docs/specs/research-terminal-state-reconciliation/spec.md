# Spec: Research Terminal State Reconciliation

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

When a Research session completes successfully, its plan checklist and final
answer must reach the active Research and Quick Chat UIs deterministically. A successful
terminal transition completes any remaining non-failed plan steps and publishes
the corresponding work surface before the completion event. The UI also treats
the final response carried by that completion event as a durable live fallback.

## Boundaries

### Always do

- Keep `session_plans` as the sole durable source for a session's operational
  plan; do not infer a plan from unrelated ward specification files.
- Preserve `failed` plan steps when terminalizing a successful session.
- Ensure the final response is durable before emitting terminal completion and
  render the response payload from `agent_completed` when it is present.
- Publish a session-scoped `surface_updated` event only when terminalization
  changes a persisted plan.

### Ask first

- Changing the WebSocket, REST, or SQLite schema.
- Changing failed/cancelled session semantics or rewriting historical plans.

### Never do

- Never add polling, retries, a second plan store, or a new dependency to mask
  event ordering.
- Never substitute the ward's newest `specs/**/plan.md` for a session plan.
- Never mark a failed plan step completed merely because the overall session
  reached successful completion.

## Testing Strategy

- Terminal plan persistence and surface publication: **TDD**, using focused
  Rust state/lifecycle tests for successful sessions with pending,
  in-progress, completed, and failed steps.
- Continuation context: **TDD**, verifying a current session plan is preferred
  over a ward's unrelated latest plan file.
- Final-response rendering: **TDD**, using Research and Quick Chat
  event-map/reducer tests that assert the final `agent_completed.result`
  appears in the completed turn.
- Regression gates: **goal-based check**, using focused Rust and Vitest suites,
  formatting, linting, and the relevant UI build.

## Acceptance Criteria

- [x] Given a successful terminal session with a current plan, when its final
  execution completes, pending and in-progress steps persist as `completed`,
  completed steps stay completed, and failed steps stay failed.
- [x] That changed terminal plan is published as the session's native
  `surface_updated` event before `agent_completed`.
- [x] Given a session plan and an unrelated newer ward spec plan, continuation
  context contains the session plan and never the unrelated ward plan.
- [x] Given a WebSocket `agent_completed` event with a non-empty `result`, the
  completed root Research or Quick Chat turn visibly renders that result even
  if the earlier `respond`/`turn_complete` event was missed.
- [x] The final answer row is durable before `agent_completed`, so a terminal
  snapshot can reconstruct it without timing-dependent omission.
- [x] Given a delegated Research continuation whose terminal response is sent
  through `respond`, its persisted assistant row and a later snapshot both use
  that final response rather than an earlier progress message.
- [x] Given duplicate terminal delivery events with the same root response,
  Research retains one completed turn and does not schedule another render.
- [x] Focused Rust and UI tests, formatting, linting, and the UI build pass
  without changing unrelated working-tree edits.

## Assumptions

- Technical: accepted plans are persisted in `session_plans` and projected to
  Research as native work surfaces (source:
  `gateway/gateway-execution/src/invoke/stream_event_processor.rs`).
- Technical: `agent_completed` carries an optional final `result`, while the
  Research event map currently discards it (source:
  `gateway/gateway-ws-protocol/src/messages.rs` and
  `apps/ui/src/features/research-v2/event-map.ts`).
- Technical: the batch writer persists assistant messages asynchronously,
  allowing a completion-triggered snapshot to race the final row (source:
  `gateway/gateway-execution/src/invoke/batch_writer.rs`).
- Product: successful completion should close remaining non-failed plan work
  and show the final response (source: user confirmation 2026-07-16).
- Process: this is a behavior-preserving defect correction with no public
  contract, schema, or dependency expansion (source: `docs/CONVENTIONS.md`).
