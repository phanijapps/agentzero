# Spec: Session-stop cancellation

- **Status:** Implementing
- **Owner:** @videogamer
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** `durable-generic-agent-tasks`, `durable-work-queue`, `durable-queue-worker-runtime`
- **Brief:** none
- **Discovery:** none
- **Contract:** none — this changes the semantics of existing WebSocket `cancel` and `session_cancelled` messages.
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing PR must match this spec, or update it. Verification must be derivable from it.

## Objective

A person who presses Stop for a Research or Quick Chat request sees that request become cancelled and receives no more work from it. A durable Research submission exposes its reserved request identity immediately after durable acceptance, so Stop is available before runtime launch. Cancellation applies whether the request is waiting in durable work storage, streaming from the model, or executing through any depth of delegated agents. It affects no other session, including another session that happens to use the same conversation correlation.

## Boundaries

### Always do

- Persist cancellation before relying on in-memory execution handles.
- Verify the requesting connection is bound to the selected session before any cancellation side effect.
- Bind a newly durable Research request to its initiating connection before returning its reserved session identity.
- Cancel queued durable work, the root execution, and every reachable delegated descendant for the selected request.
- Return the existing session-cancelled protocol event only after the cancellation request has been accepted.

### Ask first

- Changing protocol names, removing legacy `stop`, or changing cancellation behavior for non-UI callers.
- Adding a durable-work schema migration or an external queue/broker.

### Never do

- Cancel handles or durable work belonging to another session.
- Treat a sent WebSocket frame as a successful cancellation acknowledgement.
- Return internal persistence or runtime errors to the client.
- Add a new top-level dependency, protocol transport, or module boundary for this feature.

## Testing Strategy

- Session cancellation selection and descendant traversal: TDD, using runner and durable-task tests to prove the target request is cancelled while an unrelated request remains live.
- UI control flow: TDD, using hook/transport tests to prove Stop uses session-scoped cancellation and exposes a failure acknowledgement.
- Durable queued request and active delegated request: goal-based integration tests through the real gateway runner and durable task handler because the invariant crosses persistence, runtime, and WebSocket boundaries.
- User flow: visual/manual QA in the built Research and Quick Chat UI, exercising Stop while a request is queued and while delegated work is active.

## Acceptance Criteria

- [ ] **AC1 — Queued cancellation:** Given a queued Research request, when the user presses Stop after the request identity is available, the durable request is cancelled and never invokes the agent runtime.
- [ ] **AC2 — Immediate identity:** Given Research durable enqueue succeeds but runtime launch has not begun, when the gateway returns acceptance, the UI receives the reserved session identity and can issue cancellation for that request.
- [ ] **AC3 — Pre-identity control:** Given a Research turn is running but has no session identity, the UI does not offer Stop until the reserved session identity is bound.
- [ ] **AC4 — Recursive cancellation:** Given a streaming root request with nested delegated agents, when the user presses Stop, all executions belonging to that request reach a cancelled/stopped terminal state and emit no further model or delegation work.
- [ ] **AC5 — Cross-session isolation:** Given two concurrent sessions, when one is stopped, the other remains runnable and receives no cancellation signal.
- [ ] **AC6 — Same-conversation isolation:** Given two sessions share a conversation correlation, when one is stopped, cancellation selects only the persisted target session and root execution.
- [ ] **AC7 — Initiator authorization:** Given a client is not bound to a target session, when it submits cancellation for that session, the gateway rejects it before durable or in-memory state changes and emits no success acknowledgement.
- [ ] **AC8 — Confirmed UI state:** Given cancellation succeeds, when the UI receives the acknowledgement, the active turn becomes stopped/cancelled and the Stop control is no longer offered.
- [ ] **AC9 — Visible failure:** Given cancellation cannot be accepted, when the UI receives the failure, the user sees a request-level error instead of a false successful stop state.
- [ ] **AC10 — Fail-closed errors:** Given cancellation encounters a durable-store or runtime error, when the gateway reports failure, it emits a sanitized client error, records structured diagnostic context, and emits no success acknowledgement.
- [ ] **AC11 — Bounded traversal:** Given delegation registration contains a duplicate or cycle, when the root request is stopped, traversal terminates idempotently without cancelling an unrelated session.

## Assumptions

- Technical: Research enters a running UI state before its server session identity is available, and durable work is queued before runtime invocation (source: `apps/ui/src/features/research-v2/useResearchSession.ts`, `gateway/src/durable_agent_tasks.rs`).
- Technical: the existing WebSocket protocol contains `Cancel { session_id }` and `SessionCancelled { session_id }` (source: `gateway/gateway-ws-protocol/src/messages.rs`).
- Product: Stop cancels the entire request, including queued work and all delegated descendants (source: user confirmation 2026-09-03).
- Process: public UI and protocol behavior changes require the full work-loop plan, gates, and review (source: `.agents/skills/work-loop/SKILL.md`).
