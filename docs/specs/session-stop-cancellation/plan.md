# Plan: Session-stop cancellation

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

## Approach

Make session cancellation a durable, request-scoped operation. The durable task service records or settles cancellation before launch; the execution runner interrupts only handles belonging to the selected session and reaches all delegated descendants. The UI invokes this session cancellation path and waits for its explicit server acknowledgement.

## Constraints

- Preserve the existing WebSocket `cancel` / `session_cancelled` message names and HTTP gateway cancellation route.
- Preserve continuation semantics for a new user request after a cancelled session is deliberately reactivated.
- Keep cancellation scoped by persisted session identity and root execution identity; conversation IDs remain routing/correlation data, not authorization for broad cancellation.
- Respect the durable queue's existing provenance, lease, fencing, and terminal-state invariants from the constrained specs.

## Construction tests

**Integration tests:** durable queued cancellation and recursively active delegated cancellation, each with AC5/AC6 isolation and AC7 unauthorized-client rejection assertions.

**Manual verification:** start Research and Quick Chat against the local gateway; confirm Stop is absent until the request identity arrives, then press it before model output and during delegated work; confirm the selected request stops and a second request continues.

## Design (LLD)

### Design decisions

Cancellation is persisted before runtime interruption so queued work cannot start after a successful acknowledgement. Durable Research enqueue returns its reserved session identity immediately after persistence, rather than waiting for runtime readiness. The existing `Cancel { session_id }` protocol command remains the user-facing control; it becomes the canonical UI path rather than adding another stop command. Traces to: AC1–AC6.

### Data & schema

The implementation reuses durable work provenance (`session_id`, `execution_id`) and execution-state terminal statuses. It adds no new user-visible identifier and no broker. Traces to: AC1, AC3.

### Interfaces & contracts

The existing WebSocket cancel command is acknowledged only after durable and execution cancellation are accepted. The existing `SessionCancelled` server message remains the success event; cancellation failures continue through the error message channel. Traces to: AC4, AC5.

### Component / module decomposition

`DurableAgentTaskService` owns acceptance, initiator binding, and cancellation of queued Research work. `ExecutionRunner` owns scoped, recursive interruption of active handles selected by session/root-execution identity. `HttpTransport` and the Research/Quick Chat hooks own acknowledgement-aware UI state. Traces to: AC1–AC8.

### State & control flow

UI receives a reserved session identity at durable acceptance, sends `cancel`, and waits for an event. Gateway first verifies the requesting connection is the stored initiator for that session, then cancels matching durable work and persisted execution state, and finally interrupts the selected active execution tree selected by session/root-execution identity. Queued work observes the durable terminal state and exits without launch. Active work observes its cancellation signal and finalizes as cancelled. Traces to: AC1–AC8.

### Failure, edge cases & resilience

Cancellation is idempotent for already-cancelled work. A missing, unauthorized, or non-cancellable session returns a sanitized explicit failure without changing UI state; detailed diagnostics remain in structured logs. Cancellation traversal uses a visited set, terminates on duplicate/cyclic registration, and never scans/cancels unrelated handles. Traces to: AC2–AC8.

### Quality attributes (NFRs)

The selected request receives no further model/delegation work after cancellation is accepted; unrelated sessions remain unaffected. Tests exercise the persisted queued state and active runtime state, rather than only mock WebSocket frames. Traces to: AC1–AC5.

## Tasks

### T1: Durable cancellation prevents queued Research launch

**Depends on:** none

**Mode:** TDD — the durable state transition is deterministic and must be verified before runner launch.

**Touches:** `gateway/src/durable_agent_tasks.rs`, `gateway/src/durable_agent_tasks.rs` tests

**Tests:**
- `stub: true` — add red tests for AC1, AC2, AC5, and AC7 in the existing durable-task suite.

**Red stub:** `gateway/src/durable_agent_tasks.rs` unit tests; no new test module is needed because the existing module contains task-service contract tests.

**Approach:**
- Add a narrowly scoped durable-task cancellation entry point using existing work provenance session/execution fields.
- Persist and validate the initiating connection binding, then emit acceptance once the durable record is present.
- Make task inspection recognize terminal cancellation before invoking the runtime.

**Done when:** queued cancellation is durable, idempotent, and the focused gateway test passes.

### T2: Runner cancellation is scoped and recursive

**Depends on:** T1

**Mode:** TDD — recursive selection and isolation are deterministic runner invariants.

**Touches:** `gateway/gateway-execution/src/runner/core.rs`, `gateway/gateway-execution/src/delegation/registry.rs`, `gateway/gateway-execution/tests/*`

**Tests:**
- `stub: true` — add red tests for AC4, AC5, AC6, and AC11 in the existing runner core suite.

**Red stub:** `gateway/gateway-execution/src/runner/core.rs` tests; use the existing in-module handle test fixture.

**Approach:**
- Replace global-handle cancellation with a session-scoped traversal rooted at persisted session and root-execution identity.
- Reuse persisted session cancellation before signalling the selected handles.

**Done when:** the selected execution tree terminates and isolation tests pass.

### T3: Gateway and UI use acknowledged session cancellation

**Depends on:** T1, T2

**Mode:** TDD plus visual/manual QA — protocol state transitions are testable and the Stop control is a user-visible interaction.

**Touches:** `gateway/src/websocket/handler.rs`, `gateway/src/services/runtime.rs`, `apps/ui/src/services/transport/*`, `apps/ui/src/features/{research-v2,chat-v2}/*`

**Tests:**
- `stub: true` — add red transport/hook tests for AC3, AC8, AC9, and AC10 in the existing focused suites.

**Red stub:** `apps/ui/src/services/transport/http.ws.test.ts`, `apps/ui/src/features/research-v2/useResearchSession.test.ts`, and `apps/ui/src/features/chat-v2/useQuickChat.test.ts`; extend their existing focused suites.

**Approach:**
- Route Stop through the existing session cancellation command once a request identity is known.
- Verify the requesting WebSocket connection has a session binding before cancellation, and log the accepted/rejected operation with opaque identifiers.
- Return only sanitized cancellation errors to the UI and add the smallest acknowledgement correlation/state transition needed to prevent false success.

**Done when:** both UI flows send the canonical cancellation command and render the confirmed outcome.

### T4: Verify whole-request cancellation through real boundaries

**Depends on:** T1-T3

**Mode:** Goal-based integration and visual/manual QA — whole-request semantics cross the real queue, runner, WebSocket, and UI.

**Touches:** `e2e/playwright/full-mode/regressions/*`, relevant gateway tests

**Tests:**
- no stub (goal-based integration/manual QA) — add or extend a full-mode regression for AC4, AC5, AC6, and AC8.
- Run focused Rust and UI tests, workspace check/format/lint gates, and the built UI manual Stop flow for AC1–AC11.

**Approach:**
- Use the mock LLM fixture to keep the selected task active long enough to cancel.
- Assert terminal state and that an independent session, including one sharing the conversation correlation, still progresses.

**Done when:** the regression passes against the real gateway/UI boundary and gates are green.

## Rollout

Ship as a backward-compatible behavior correction: existing clients retain `stop`, while the current UI uses session cancellation. Rollback is reverting the code change; no data migration or external infrastructure is involved.

## Risks

- A durable-work cancellation API may not expose the required session-scoped operation, requiring a bounded store-level extension.
- A handle registry may lack enough session metadata for recursive selection; any metadata addition must remain internal and be covered by isolation tests.
- The full-mode mock fixture may not naturally hold nested work open; the test fixture must make the timing deterministic rather than sleep-based.

## Changelog

- 2026-09-03: initial plan for full-request cancellation.
