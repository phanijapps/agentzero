# Plan: Research Submit Visibility

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Persist the root message during invocation setup, before the session-ready
callback, lifecycle events, and intent analysis. Keep the execution stream
responsible for assistant/tool messages only. The browser sends its generated
message ID through `metadata.client_message_id` in the existing optional invoke
envelope. The gateway extracts only that string into the dedicated
`ExecutionConfig.client_message_id` field, validates it at persistence time,
and uses it as the durable message ID. The UI merges snapshots only when that
exact ID appears. This adds no endpoint, event variant, or database schema.

## Constraints

- Preserve `conversations.db` schema and message ordering.
- Do not add an endpoint, event variant, dependency, or abstraction layer.
- Treat user-provided prompt content as opaque data; no new logging or exposure.
- A failed durable append is fail-closed: no session-ready callback, lifecycle
  event, recall, intent analysis, or model invocation can follow it.

## Construction tests

**Integration tests:** a runner test uses the session-ready callback to read
the message store and proves the prompt is present before the callback returns.
A history test proves the current row is filtered from prior history. Reducer
tests drive `APPEND_USER` then stale/fresh `HYDRATE` snapshots, including
identical consecutive prompts and an invoke failure.

**Manual verification:** submit a request from `/research` in a new and an
existing session; verify the user bubble never disappears before or during
intent analysis and delegation.

## Design (LLD)

### State & control flow

The gateway owns the durable handoff: append the root prompt synchronously in
`begin_setup`, then return the session ID to the callback, emit lifecycle
events, and run intent analysis. It carries the persisted row ID into phase 2
and removes only that row from history; the engine still receives the prompt as
its separate current message. The UI owns a session-scoped submitted identity:
its UUID is supplied in invoke metadata and snapshot hydration merges only a
server turn with the same ID, scoped to the current session and root execution.
The identity remains until the next submission or reset, even after the durable
row has replaced the rendered optimistic turn, so out-of-order snapshots stay
safe. Traces to: AC1–AC5.

### Failure, edge cases & resilience

A failed durable append returns a generic invoke error before publication; the
UI retains the submitted bubble and marks the request error. A duplicate or
late snapshot remains safe because matching uses the opaque message ID, never
prompt content or timestamps. The reducer retains the latest submitted ID for
the session so a stale-after-fresh snapshot cannot erase a confirmed turn. Each
client retry mints a new ID and is a new submission; transport-level at-most-
once retries are outside this fix. Traces to: AC2, AC4–AC6.

## Tasks

### T1: Root prompts are durable before lifecycle publication

**Depends on:** none

**Touches:** `gateway/gateway-execution/src/runner/{core.rs,invoke_bootstrap.rs,execution_stream.rs}`, `gateway/src/{websocket/handler.rs,services/runtime.rs}`, focused gateway-execution and gateway tests

**Tests:**

- TDD: prove the user message exists in the message store inside the
  `on_session_ready` callback, before it can snapshot. Covers AC3.
- TDD: prove the history given to the engine excludes the current durable row
  while retaining prior rows. Covers AC5.
- TDD: prove append failure returns before the callback and leaves no route to
  lifecycle, recall, intent, or model work. Covers AC6.
- TDD: prove `metadata.client_message_id` becomes the durable root
  `messages.id`, while absent or invalid values receive a generated server ID.

**Approach:**

- Carry the browser UUID through existing invoke metadata and use it for the
  root message row. Extract only the bounded candidate value in the WebSocket
  handler/runtime path into `ExecutionConfig.client_message_id`, then validate
  it immediately before persistence; general request metadata can never become
  a message-store key. Server-originated, absent, or invalid values receive a
  generated ID.
- Move the root user-message append from the spawned execution stream into
  `begin_setup`, before it returns `PartialSetup` to the callback.
- Carry that message ID to phase 2, filtering it from the replayed history.
- Remove only the now-duplicate stream append; retain its assistant and tool
  persistence behavior.

**Done when:** the gateway regression test is green and no execution writes a
duplicate root user message.

### T2: Stale snapshots retain the submitted Research turn

**Depends on:** T1

**Touches:** `apps/ui/src/features/research-v2/{types.ts,reducer.ts,useResearchSession.ts,*.test.ts}`, `apps/ui/src/services/transport/{interface.ts,http.ts,http.ws.test.ts}`

**Tests:**

- TDD: a stale snapshot after submit preserves the visible optimistic request.
- TDD: a fresh snapshot containing the same durable message ID results in
  exactly one user turn, including two identical prompt texts. Covers AC1,
  AC2, and AC4.
- TDD: a delayed stale snapshot after that fresh confirmation still preserves
  one matching turn. Covers AC2 and AC4.
- TDD: an invoke failure leaves the optimistic turn visible in terminal error
  state and a late stale snapshot cannot erase it. Covers AC6.
- TDD: the WebSocket invoke serializer sends the named `client_message_id`
  metadata field. Covers AC4.

**Approach:**

- Add only the pending message ID and session/root-execution scope required
  for deterministic hydration merging.
- Pass that ID as the sixth optional `executeAgent` argument and serialize it
  only as `metadata.client_message_id`; no prompt text enters metadata.
- Retain only the latest submitted metadata until the next submission or
  `RESET`, even after a matching snapshot, so stale-after-fresh snapshots are
  safe. On failure it protects the error bubble from in-flight stale snapshots.

**Done when:** focused Research tests demonstrate the complete stale-to-fresh
snapshot transition without lost or duplicate request bubbles.

### T3: Verify the submission journey and record the result

**Depends on:** T1-T2

**Touches:** `docs/specs/research-submit-visibility/*`, `docs/specs/README.md`

**Tests:**

- Goal-based: run `cargo test -p gateway-execution
  begin_setup_persists_the_root_user_message_before_lifecycle_events`, `cargo
  test -p gateway-execution current_prompt_is_excluded_from_prior_history`,
  `npm test -- --run src/features/research-v2/reducer.test.ts
  src/features/research-v2/useResearchSession.test.ts`, `npm run lint`, `npm
  run build`, `cargo fmt --check`, and `cargo check -p gateway-execution -p
  gateway -p gateway-ws-protocol`.
- Visual/manual QA: submit from both a new and an existing Research session.

**Approach:**

- Update acceptance criteria and plan status with the verification evidence.
- Preserve unrelated working-tree changes.

**Done when:** all gates pass and the regression contract is marked Complete.

## Rollout

Ship as a direct compatibility-preserving bug fix. No migration, flag, or
rollout sequencing is required; rolling back restores the previous submit
timing only.

## Verification

Automated verification completed on 2026-07-13:

- `git diff --check` and `cargo fmt --all -- --check`
- focused gateway-execution ordering, client-ID validation, and prior-history
  tests; focused gateway WebSocket metadata test; and `cargo check -p
  gateway-execution -p gateway -p gateway-ws-protocol`
- 84 focused Research reducer/hook/transport Vitest tests
- `npm run lint` (no errors; existing repository warnings only) and `npm run
  build`

Manual follow-up: restart the daemon and submit a new Research request in the
running application. The request bubble must remain visible through “analyzing
intent…” and any delegation; this intentionally uses the operator's configured
model and is not executed by the automated suite.

## Risks

- Moving persistence earlier must not cause a second user row when the stream
  starts or a second model prompt in history.
- Snapshot merging must match only the submitted durable row and not collapse
  genuine repeated prompts.

## Changelog

- 2026-07-13: initial plan.
- 2026-07-13: added callback ordering, exact durable-ID correlation,
  single-prompt history, and fail-closed requirements after design review.
- 2026-07-13: shipped with stale-after-fresh snapshot protection, terminal
  append-failure coverage, and automated gate evidence.
