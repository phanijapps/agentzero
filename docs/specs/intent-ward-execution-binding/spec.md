# Spec: Intent Ward Execution Binding

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Contract:** none
- **Shape:** service

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

When intent analysis selects an existing ward or names a new domain ward for a
new Research session, work must use that named workspace rather than silently
falling back to `scratch`. Existing wards are bound during bootstrap; a safe
new-ward recommendation is retained as the root agent's mandatory first
`ward(action="create")` call. That call persists the binding for subsequent
delegation and procedure execution.

## Boundaries

### Always do

- Bind only an authoritative `use_existing` recommendation to a session that
  has no persisted ward, before the root executor is built.
- Accept a model-derived ward name only after exact canonical matching against
  the vault's canonical regular, non-symlinked ward inventory. Verify both the
  `wards/` root and selected child immediately before use. Only that canonical
  ID may reach persistence, events, executor state, or file tools.
- Atomically claim an empty session ward and use the returned effective value;
  a concurrent session invocation must never overwrite another binding.
- Persist the binding before emitting the existing `WardChanged` event and
  before any root or delegated tool can use the ward context. Emit that event
  only when this invocation successfully claims the ward.
- Treat a failed persistence write as an invoke setup failure; never run an
  executor with a ward that artifacts cannot subsequently verify. Crash the
  started root execution/session, remove its handle, and emit a safe,
  correlated error before returning the setup failure.
- Keep legacy session-state data truthful: a recommendation alone is not an
  active ward, including when it appears in generic intent metadata.
- Write intent/prompt context facts only after an effective persisted ward is
  known. Leave a `create_new` session unbound until the existing ward tool
  explicitly creates or enters its workspace; its durable execution log and
  prompt history remain the intent source rather than writing a speculative
  ward-scoped duplicate.
- Preserve a safe, non-`scratch` `create_new` ward name in intent metadata and
  the root prompt. The required explicit ward-tool call must precede any
  `run_procedure` or delegation instruction in that prompt, so the root has
  the correct workspace setup sequence.
- Prove the binding and no-false-ward behavior with focused Rust tests.

### Ask first

- Binding `create_new` recommendations automatically rather than retaining the
  existing explicit ward-creation flow.
- Changing the public WebSocket, REST, SQLite schema, or artifact API
  contracts.
- Repairing historical scratch files or creating artifact records for them.

### Never do

- Never restore a browser-side or server-side raw-path artifact fallback.
- Never override a ward already persisted on an existing session.
- Never treat the first automatic recall as ward-scoped in this correction;
  intent analysis currently depends on that unscoped recall and its ordering is
  an explicit non-goal.
- Never add a dependency, persistence table, top-level module, or a second
  ward-routing mechanism for this correction.

## Testing Strategy

- Intent binding: **TDD**. Focused bootstrap and state-service tests prove a
  canonical `use_existing` recommendation atomically claims the selected ward,
  publishes the existing event only for the successful claimant, and reaches
  root tool context before executor construction. Invalid, missing, symlinked,
  `create_new`, and already-bound cases remain unbound or unchanged as
  appropriate.
- Session state: **TDD**. A state-builder test proves intent metadata alone
  cannot report an active ward when `sessions.ward_id` is absent.
- Downstream propagation: **TDD**. Focused integration coverage proves an
  artifact declaration and delegated execution resolve the claimed persisted
  ward, not `scratch`.
- Regression gates: **goal-based check**. Run focused gateway-execution tests,
  Rust formatting and check; run the broader workspace check only when the
  pre-existing Engram adapter compile failure is resolved.

## Acceptance Criteria

- [x] Given a new session with no active ward and a canonical authoritative
  `use_existing` recommendation, when bootstrap completes intent analysis,
  one atomic claim persists the selected ward and root executor context equals
  the effective persisted ward.
- [x] Invalid, missing, absolute, separator-containing, dot, or symlinked
  model-derived ward names cannot create a session binding, event, executor
  context, or filesystem access.
- [x] Given that bound session, when a subagent or an artifact declaration is
  processed, they resolve the same persisted ward instead of `scratch`.
- [x] Given an intent log with a ward recommendation but an empty persisted
  ward, session state exposes no active ward.
- [x] A previously bound session retains its persisted ward, and a
  `create_new` recommendation does not bypass explicit ward creation. Two
  concurrent claims use one effective persisted ward without overwriting.
- [x] The existing `WardChanged` event communicates only a successful claim;
  a binding failure crashes the root session/execution, clears its active
  handle, and exposes no storage internals. No endpoint, event schema,
  database schema, or dependency is added.
- [x] Focused tests and stated mechanical checks pass without changing
  unrelated working-tree edits.
- [x] Given a safe `create_new` Research recommendation, the exact named ward
  reaches the root's mandatory create instruction; `scratch` is reserved for
  the fast Quick Chat surface and the procedure recommendation follows the
  ward setup instruction.

## Assumptions

- Technical: `begin_setup` obtains `ward_id` from the session before intent
  analysis, and `create_executor` passes that unchanged value into
  `ExecutorBuilder::build` (source:
  `gateway/gateway-execution/src/runner/invoke_bootstrap.rs`).
- Technical: `ExecutorBuilder` seeds root tool context from its `ward_id`
  argument; delegated executors and artifact handling read the persisted
  session ward (source: `gateway/gateway-execution/src/invoke/executor.rs`,
  `gateway/gateway-execution/src/delegation/spawn.rs`, and
  `gateway/gateway-execution/src/invoke/stream_event_processor.rs`).
- Technical: the existing `WardChanged` gateway event has sufficient fields
  for a successful initial binding (source: `gateway/gateway-events/src/lib.rs`).
- Technical: the existing `update_session_ward` write is unconditional, so
  this correction needs a compare-and-set operation rather than a read then
  update sequence (source: `services/execution-state/src/repository.rs`).
- Product: an existing ward selected by intent must be the actual execution
  workspace, including for a simple request that later creates a file (source:
  user confirmation 2026-07-15).
- Process: this is a behavior-preserving defect correction with no public
  contract or schema expansion (source: existing
  `docs/specs/research-submit-visibility/` precedent).
