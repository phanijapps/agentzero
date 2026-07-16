# Spec: Research Submit Visibility

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Keep a Research request visibly present from the instant it is submitted until
the final response. Intent analysis, session binding, REST reconciliation, and
subagent delegation must not temporarily remove the user request or make it
appear only after a later activity event.

## Boundaries

### Always do

- Persist the root user message before an intent-analysis snapshot can observe
  the session — specifically before `on_session_ready` receives its ID.
- Preserve a client-created pending user turn when a session snapshot does not
  yet contain its server-persisted counterpart, including when an older
  in-flight snapshot resolves after a newer confirming snapshot.
- Correlate the optimistic turn with its durable row using a client-generated
  message ID carried as `metadata.client_message_id` in the existing optional
  WebSocket invoke envelope. The WebSocket handler copies only that value into
  the dedicated `ExecutionConfig.client_message_id` field; do not infer
  identity from content or timestamps.
- Treat invoke metadata as untrusted: accept only a bounded `msg-<UUID>`
  client-message ID. For absent or invalid values, generate a server ID rather
  than using arbitrary general metadata as a database primary key.
- Exclude precisely that current durable row from the LLM's prior-history
  replay, because the engine receives it separately as the current prompt.
- Retain the current event ordering and the existing append-only conversation
  store.
- Prove the behavior with focused Rust and UI regression tests.

### Ask first

- Changing the WebSocket event schema or adding a new client/server API.
- Changing intent-analysis behavior, model selection, or delegation policy.
- Changing conversation retention, migration, or database schema.

### Never do

- Never hide, discard, or duplicate a user request during normal Research
  submission.
- Never delay intent analysis merely to mask a UI race.
- Never add a new runtime dependency, persistence table, service boundary, or
  top-level module for this fix.

## Testing Strategy

- Gateway persistence ordering: **TDD**. A runner-level test will prove the
  root user message is durable before the session-ready callback can expose the
  session to a REST snapshot. A history test proves the prompt reaches the
  model once while earlier conversation stays available.
- Research snapshot reconciliation: **TDD**. Reducer or hook-level tests will
  prove a stale snapshot cannot erase a locally pending submitted request and
  that the later durable row with the same message ID replaces its rendered
  optimistic turn without duplication, including identical consecutive
  prompts. The client retains that latest ID until the next submission or
  reset, so an older snapshot that arrives after confirmation stays harmless.
- User-visible journey: **visual/manual QA**. Submit a prompt in both a new and
  an existing Research session; the prompt remains on screen while the status
  says intent analysis is running and through delegation.
- Regression gates: **goal-based check**. Run focused Rust and Vitest suites,
  UI typecheck/lint, formatting, and the relevant workspace check.

## Acceptance Criteria

- [x] Given a new or existing Research session, when the user submits a prompt,
  the request is visible immediately and remains visible while intent analysis
  runs.
- [x] A REST session snapshot taken before the submitted message has been
  persisted cannot remove that local request from the Research timeline.
- [x] The gateway persists the root user message before it invokes the
  session-ready callback or emits lifecycle events that allow Research to bind
  and snapshot the session.
- [x] When a later snapshot contains the persisted request, the UI has one
  corresponding user turn, not a duplicate optimistic and persisted turn.
- [x] The model receives the current request once (as its current prompt),
  while the persisted prior conversation remains in history.
- [x] If root-message persistence fails, the invocation stops before callback,
  lifecycle event, intent analysis, recall, or model work; the existing user
  bubble remains in an error state.
- [x] Focused regression tests and the stated quality gates pass.

## Assumptions

- Technical: `ResearchSessionState` currently appends a local user turn before
  invoking the gateway, but `HYDRATE` replaces all turns (source:
  `apps/ui/src/features/research-v2/useResearchSession.ts`,
  `apps/ui/src/features/research-v2/reducer.ts`).
- Technical: gateway execution emits `AgentStarted` and runs intent analysis
  before the spawned execution stream queues the root user message for its
  periodic batch writer (source:
  `gateway/gateway-execution/src/runner/core.rs`,
  `gateway/gateway-execution/src/runner/execution_stream.rs`).
- Product: a submitted Research request must remain visible before subagent
  activity (source: user confirmation 2026-07-13).
- Process: this is a behavior-preserving defect fix with no public contract or
  schema expansion. The existing optional invoke `metadata` field carries the
  client message ID, so the WebSocket envelope and database schema stay
  compatible (source: `docs/CONVENTIONS.md`).
