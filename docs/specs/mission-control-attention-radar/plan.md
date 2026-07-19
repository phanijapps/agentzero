# Plan: Mission Control Attention Radar

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

> **Plan contract:** this is the implementation strategy. It can change as the
> implementation teaches us something; material changes are recorded below.

## Approach

Add a compact Mission Control subscription to the existing gateway WebSocket
and reuse the existing subscription manager rather than creating a polling
service or second socket. A private, in-memory `MissionControlStream` projection
shares one serialization gate with lifecycle projection: it hydrates bounded
state once, clones a snapshot and its watermark under that gate, writes the
snapshot to a pending subscriber before making it live, and then maps later
lifecycle events into summary-only deltas. The UI owns a bounded indexed Radar
reducer, treats each snapshot epoch as authoritative, reconnects by requesting
a snapshot, and expands detail for the one focused mission by default.
The new page is a Mission Control-only composition using existing tokens and
the approved Radar hierarchy.

## Constraints

- The bounded summary and selected-detail patterns from
  [`mission-control-performance`](../mission-control-performance/spec.md) stay
  in force.
- The implementation must follow the Quiet Instrument token and accessibility
  vocabulary without applying its work to other routes in this feature.
- The existing dirty Mission Control/session-plan work is in scope to preserve
  and integrate; unrelated dirty files are not reformatted or reverted.
- No dependency, background worker, durable dashboard projection, or raw trace
  broadcast is introduced.

## Construction tests

- Integration: a WebSocket client that does not send `mission_control_subscribe`
  receives no Mission Control message; an explicit subscriber receives a
  snapshot and a later delta.
- Manual verification: open `/mission-control` at approximately 1440px, 900px,
  and 390px; verify live state, focus selection, Open Loops action access,
  Research deep link, and no unintended horizontal document scroll.

## Design (LLD)

### Design decisions

- Reuse a reserved internal subscription channel in `SubscriptionManager` for
  explicit Mission Control subscribers. It keeps recipient selection O(number
  of subscribers), rather than broadcasting lifecycle traffic to every socket.
- Snapshot replay is intentionally fresh-only in this pass. The server's
  authoritative bounded query resolves reconnect gaps and daemon restarts;
  each snapshot carries a server epoch and always replaces client state.
- The private projection's one serialization gate is also the snapshot
  consistency boundary. Lifecycle state must commit before its event is
  published; snapshot cloning and event sequence assignment cannot interleave.
  A snapshot therefore represents the projection at its included watermark;
  later deltas are strictly newer. This is not a durable projection or history
  store.
- The dashboard stream carries summary/activity fields only. Existing selected
  session APIs and subscriptions remain the trace authority.

Traces to: AC 1–5, 7–8 · `contracts/asyncapi/mission-control.yaml`.

### Data & schema

- `MissionControlSnapshot` contains a server epoch, a sequence watermark, at
  most 30 active and 20 recent terminal `MissionControlSessionSummary` rows,
  and at most 20 fixed-shape activities.
- `MissionControlDelta` contains a sequence number, one changed summary, and a
  fixed activity kind (`started`, `progress`, `delegated`, `completed`,
  `failed`, or `plan_updated`). It contains no message, tool-argument,
  tool-result, or event-derived free text. A summary carries authoritative
  `last_activity_at`, `attention_kind`, and `plan_revision` fields.
- Browser state is a `Map` keyed by root execution id, a most-recent activity
  array capped at 20 in the reducer, connection state, snapshot epoch/sequence,
  and selected focus id. Every upsert deterministically keeps only the 30
  highest-ranked active rows and 20 newest terminal rows.
- Before a summary enters the projection, display-only text (`title`) is
  UTF-8-safely truncated to the contract's byte limit. Stable identities
  (`conversation_id`, `root_execution_id`, `root_agent_id`, and activity
  `root_execution_id`) are never truncated: a record outside its protocol
  bound is omitted and emits only fixed internal telemetry. The fixed 30/20/20
  aggregate and field budgets fit within the 64 KiB encoded server-frame
  ceiling.

Traces to: AC 1–3, 5, 7–8 · `contracts/asyncapi/mission-control.yaml`.

### Interfaces & contracts

- Client frame: `mission_control_subscribe` (and
  `mission_control_unsubscribe`) over the existing `/ws` connection.
- Server frames: `mission_control_snapshot`, `mission_control_delta`, and
  `mission_control_subscription_error`; snapshots include a bounded initial
  activity feed.
- The dedicated frame decoder uses closed serde variants with an inbound frame
  byte limit. Frames have no additional fields; fixed error codes, not internal
  error messages, are returned to the sender.
- Existing REST endpoints remain on-demand history/detail mechanisms, not a
  Radar refresh mechanism.

Traces to: AC 1–5, 7–8 · `contracts/asyncapi/mission-control.yaml`.

### Component / module decomposition

- `useMissionControlRadar` owns the transport lifecycle and pure reducer.
- `MissionControlPage` owns selected focus and lays out the header, indicator
  strip, mission radar, focused mission, and a right-hand live-operations /
  system-posture stack.
- Existing `SessionDetailPane` is the default deep inspector for the one
  focused mission, while remaining absent for every unselected row.

Traces to: AC 1–5.

### State & control flow

1. Page mounts → connects transport → subscribes once → applies a fresh-epoch
   snapshot and then later deltas in sequence.
2. A summary delta updates one map entry, activity feed, and derived attention
   ranking; it never refetches the list. A reducer eviction pass maintains the
   active/recent-terminal quotas after every delta.
3. A delta is accepted only when its epoch equals the current snapshot epoch
   and its sequence is newer; only a snapshot may replace the epoch. A single
   deadline timer is scheduled for the earliest running session's
   `last_activity_at + five minutes`; when it fires it recomputes local stale
   ranking only and schedules the next deadline. It does not fetch or poll.
4. Transport reconnection invokes the same subscribe and replaces indexed state
   on the next authoritative snapshot, even when its sequence restarted.
5. Focus changes keep the focused mission inspector expanded for that one
   selected session. A `plan_updated` delta refreshes only its
   selected-token/current-plan endpoint.

Traces to: AC 1–5.

### Behavior & rules

- Attention order is deterministic: failed/crashed, stale-running, then
  active/recent. The right column is reserved for live operations and system
  posture rather than an autonomy decision-thread queue.
- A session is stale-running after five minutes without a lifecycle/activity
  delta. The single deadline timer is driven from authoritative
  `last_activity_at`, so a quiet running session becomes stale without a fetch
  or repeating clock.
- Terminal sessions remain in the bounded recent set but do not outrank an
  actionable session.

Traces to: AC 2–3.

### Failure, edge cases & resilience

- A subscription error leaves the current snapshot visible with a clear
  degraded indicator and a manual reconnect action; it disables automatic
  subscription retries until the WebSocket reconnects.
- An invalid, mismatched-epoch, or out-of-order delta is ignored. A transport
  reconnect obtains a fresh snapshot, which is the only event allowed to
  replace the applied epoch.
- Repeated subscribe is idempotent. The server retains one membership and at
  most one in-flight snapshot query per connection; disconnect cleanup removes
  that state. Browser reconnection uses the established capped backoff.
- The server marks a client pending, then acquires the projection gate to clone
  state and watermark atomically. While holding that gate it performs one
  nonblocking enqueue to the connection's ordered writer and marks the client
  live; it releases the gate before any socket I/O. Later projection work then
  receives a strictly greater sequence. A stalled writer cannot delay another
  subscriber's delta. This makes the handoff race-free without a per-client
  replay journal.
- If snapshot creation fails, the server sends a fixed error code with no
  database/debug detail; it does not expose a broad fallback event stream.

Traces to: AC 1, 3, 7–8.

### Quality attributes (NFRs)

- Initial Radar data is bounded to 50 summary rows and subsequent messages are
  one summary-sized delta, independent of historical session count. Every
  server frame has a fixed encoded-byte ceiling.
- The page preserves semantic controls, labels, focus visibility, and a
  responsive single-column fallback.

Traces to: AC 1, 5–6, 8.

### Dependencies & integration

- Gateway WebSocket protocol and `SubscriptionManager` provide event transport
  and recipient routing.
- The private Mission Control projection is an in-process, bounded read model:
  it hydrates from `StateService` once and serializes later event projections;
  it is discarded on daemon restart and never becomes a persistence layer.
- React `HttpTransport` uses the already-open WebSocket and exposes a
  Mission-Control-specific subscription wrapper.

Traces to: AC 1–5, 7–8.

## Tasks

### T1: Mission Control WebSocket contract and bounded snapshot routing

**Depends on:** none

**Touches:** `contracts/asyncapi/mission-control.yaml`, `gateway/gateway-ws-protocol/src/messages.rs`, `gateway/src/websocket/handler.rs`, `gateway/src/websocket/subscriptions.rs`, `gateway/src/websocket/*test*`

**Tests:**

- Add protocol serialization/deserialization tests for subscribe, snapshot,
  delta, unsubscribe, and fixed-code subscription errors. Assert unknown or
  oversized client frames are rejected (AC 1, 7–8).
- Add routing tests proving an unsubscribed client sees no mission messages and
  a subscriber sees one snapshot plus relevant summary delta. Add repeated
  subscribe/disconnect tests that prove one membership and no snapshot storm
  (AC 1, 7–8).
- Add an ordered handoff test that emits a lifecycle event while snapshot
  clone is pending and proves the client receives snapshot then that delta.
  Add state-mutation tests immediately before and after the projection-gate
  watermark, proving neither is omitted and the snapshot never contains a
  later projected revision than its watermark;
  add an epoch-reset test that proves a fresh reconnect snapshot replaces prior
  higher-sequence state (AC 1, 3).
- Add a state-service/projection fixture with more than 30 active and 20
  terminal rows, proving snapshot membership/order is exactly the 30 highest
  attention active rows plus 20 newest terminal rows (AC 1–2).
- Add a maximum-size fixture with multibyte title input and 20 activities,
  proving UTF-8-safe display-title truncation, exact identity preservation,
  activity `root_execution_id` membership, and an encoded snapshot at or
  below 65,536 bytes (AC 1, 8).
- Add a stalled-writer test proving the nonblocking snapshot enqueue does not
  delay a delta for another subscriber (AC 1, 7–8).

**Approach:**

- Define the event contract and add additive protocol messages.
- Reuse a reserved subscription key and existing scope manager for explicit
  recipient routing.
- Create one bounded private projection from the existing state-service summary
  query, then map lifecycle events to safe, summary-only deltas with fixed
  activity kinds and strictly increasing projection revisions.
- Bound frame decode/encode size and return fixed error codes only to the
  requesting WebSocket session.

**Done when:** target Rust protocol/routing tests pass and no non-subscriber
receives Mission Control payloads.

### T2: Browser Radar transport, reducer, and reconnection semantics

**Depends on:** T1

**Touches:** `apps/ui/src/services/transport/{types.ts,interface.ts,http.ts}`, `apps/ui/src/features/mission-control/useMissionControlRadar.ts`, `apps/ui/src/features/mission-control/*test.tsx`

**Tests:**

- Add reducer tests for snapshot replacement, ordered upsert, stale-delta
  rejection within an epoch, mismatched-epoch rejection, attention ordering, bounded active/terminal
  eviction, and bounded activity history (AC 1–3).
- Add a fake-clock test that a quiet running session becomes stale at its one
  deadline without a network request, plus a snapshot fixture containing its
  authoritative attention/activity fields (AC 2).
- Add transport tests showing exactly one subscribe per mounted hook and a new
  snapshot request after reconnect, with no interval timers or automatic retry
  following a subscription error (AC 1, 3, 8).

**Approach:**

- Add typed stream messages and an explicit transport subscription API.
- Implement a pure state reducer plus a hook that translates connection and
  Mission Control frames into state.
- Keep existing REST detail APIs available for user-initiated inspection only.

**Done when:** focused hook/reducer tests pass and the old Mission Control
auto-refresh invocation is absent.

### T3: Responsive Attention Radar composition and scoped styling

**Depends on:** T2

**Touches:** `apps/ui/src/features/mission-control/{MissionControlPage.tsx,SessionDetailPane.tsx}`, `apps/ui/src/styles/{theme.css,components.css}`, `apps/ui/src/features/mission-control/*test.tsx`

**Tests:**

- Update page tests to assert Radar loading, empty, degraded, focused, live
  operations, and system-posture states by visible labels/controls (AC 2, 4).
- Add a rendering test that keyboard selection changes the visibly focused
  mission and exposes the existing deep inspection action. Assert that focus
  alone makes no detail/trace request, while inspect makes one scoped detail
  request (AC 4).
- Add a plan-update rendering test that refreshes current plan once without a
  trace request when the inspected mission's revision changes (AC 5).
- Record desktop/tablet/narrow visual QA results (AC 6).

**Approach:**

- Replace the legacy list/detail grid with the approved Radar hierarchy while
  retaining the deep inspector as an explicit lower-priority detail surface.
- Add Mission Control-scoped semantic token aliases and BEM CSS only; do not
  change another page.
- Make the three-column layout collapse to two and then one column while
  retaining all functional sections.

**Done when:** Mission Control visibly matches the approved Radar structure,
all existing actions remain reachable, and responsive/manual checks pass.

### T4: Full regression gates and implementation review

**Depends on:** T1-T3

**Touches:** `docs/specs/mission-control-attention-radar/*`, `docs/specs/README.md`

**Tests:**

- Run targeted Rust tests/checks, UI tests, lint, and production build (AC 9).
- Run an adversarial code review and a WebSocket/input-boundary security review
  against the final diff; resolve actionable findings (AC 7–9).

**Approach:**

- Verify the contract, routing, and UI from narrow tests outward.
- Update this plan/spec if implementation changes any observable behavior.

**Done when:** required gates and reviews are clean, or each pre-existing
blocker has exact command output and scope recorded.

## Rollout

- **Delivery:** additive WebSocket protocol plus a self-contained Mission
  Control consumer; older clients ignore the new messages and retain their
  existing subscriptions.
- **Infrastructure:** none. The current daemon and WebSocket connection host
  the feature.
- **Deployment sequencing:** ship server protocol/snapshot support before or
  together with the UI. The UI can show a degraded connection state if talking
  to an older daemon, rather than silently polling.
- **Rollback:** revert the UI consumer; the additive protocol frames are safe
  for older clients to ignore.

## Risks

- Event ordering and database write timing may make a just-created summary
  briefly unavailable. The handler must skip that delta and the next snapshot
  reconciles it rather than inventing partial session data.
- The existing page's in-progress session-plan and autonomy changes may overlap
  the Radar composition. Preserve those changes and test the visible plan/open
  loop paths specifically.
- Matching the mockup too literally could reintroduce decorative effects that
  conflict with Quiet Instrument; prioritize its type hierarchy, density,
  surfaces, and responsive behavior while keeping effects restrained.

## Changelog

- 2026-07-15: initial plan; selected a bounded snapshot + delta stream over a
  new projection store or dashboard polling.
- 2026-07-15: hardened the event contract after secure-design review: closed
  frames, fixed activity/error vocabulary, size limits, idempotent membership,
  and reconnect/subscription-storm controls.
- 2026-07-15: specified the gated in-memory projection as the consistent
  snapshot/watermark authority, plus deterministic 30/20 retention and
  byte-budgeted wire fields after adversarial plan review.
- 2026-07-15: user directed the focused inspector to be expanded by default;
  narrowed the center column and retained its independent scroll boundary.
- 2026-07-15: user removed autonomy decision threads from this surface; the
  right column now stacks live operations above system posture.
