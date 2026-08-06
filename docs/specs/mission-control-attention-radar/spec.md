# Spec: Mission Control Attention Radar

- **Status:** Shipped
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/asyncapi/mission-control.yaml`](../../../contracts/asyncapi/mission-control.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Replace the polling-based Mission Control dashboard with an event-driven
Attention Radar. A user can immediately see the few sessions that need their
attention, inspect the focused mission's live plan, delegation, and recent
activity, and use its focused tool trace when needed. The page uses the
approved dark, precise Radar visual language on desktop and narrow screens
without changing any other route in this feature.

## Boundaries

### Always do

- Keep the initial mission snapshot bounded to active sessions plus a small,
  recent terminal set; load message and tool detail only for the focused
  session.
- Deliver Radar state through one explicit WebSocket subscription, with a
  snapshot followed by normalized deltas; reconnecting clients receive a fresh
  snapshot rather than falling back to timer polling.
- Validate the fixed Mission Control frames as closed, size-bounded messages;
  subscription membership and snapshot work are idempotent per connection.
- Show direct access to the current plan and selected-session Research trace in
  the default focused-mission view, while using the right column for live
  operations and system posture.
- Use existing React, TypeScript, Rust, WebSocket, Lucide, and shared semantic
  design tokens; all new UI styles use semantic BEM classes.

### Ask first

- Adding a package dependency, a worker, a persistent projection store, or a
  new top-level module/service boundary.
- Applying this visual system to another route, changing session retention, or
  exposing raw tool arguments/results in the dashboard-wide stream.
- Replacing an existing pause/cancel/retry runtime contract.

### Never do

- Never poll the bounded Radar snapshot or selected-session token data on a
  timer while the page is open.
- Never load all sessions, all traces, or raw tool payloads into the Radar.
- Never remove the existing selected-session detail, Open Loops actions, or
  Research deep link merely to simplify the new layout.
- Never introduce a new UI framework, component library, route, or inline
  styling boundary.

## Testing Strategy

- **TDD:** the Radar reducer and WebSocket protocol handling have precise
  snapshot/delta/reconnect invariants that unit tests can enforce.
- **TDD:** Rust protocol serialization and subscription routing tests verify
  bounded snapshots and that only explicit Radar subscribers receive deltas.
- **Goal-based:** UI lint, TypeScript production build, Rust formatting, and
  targeted crate checks prove the existing application compiles against the
  added event surface.
- **Visual/manual QA:** desktop, tablet, and narrow-screen checks verify the
  approved information hierarchy, live connection state, keyboard focus, and
  non-overflowing layout.

## Acceptance Criteria

- [x] Given Mission Control opens, when the WebSocket connects, the page
  renders one bounded Radar snapshot and then applies mission deltas without a
  `setInterval`-based list or selected-token refresh.
- [x] Given an active, stalled, or failed session, when its normalized state
  changes, the Radar ranks and labels its attention reason and updates the
  focused mission without fetching the complete history; durable decision
  threads remain a separate visible queue.
- [x] Given a disconnect or daemon restart, when the transport reconnects, the
  Radar requests and renders an authoritative fresh-epoch snapshot without
  duplicate rows or stale optimistic state.
- [x] Given the Radar chooses or a user focuses a mission, its current plan,
  delegation/activity summary, and selected-session trace are expanded by
  default in a separately scrollable focused-mission panel; raw tools are
  loaded only for that one focused session.
- [x] Given an inspected mission receives a `plan_updated` delta, when its
  plan revision changes, the visible current plan refreshes once from the
  selected-session token endpoint without fetching its trace history.
- [x] Given desktop, tablet, and narrow mobile widths, when the route renders,
  the header, KPI strip, Radar, focused mission, attention queue, and operation
  feed remain readable, keyboard-operable, and free of horizontal page
  overflow.
- [x] Given a client that did not explicitly subscribe to Mission Control,
  when lifecycle events occur, it receives no Radar snapshot or delta.
- [x] Given malformed, oversized, unknown-field, or repeated Mission Control
  frames, when the server handles them, it rejects only the sender with a fixed
  code, performs at most one snapshot query in flight per connection, and
  exposes no raw event/database text.
- [x] Given this feature branch, when its targeted UI/Rust tests, lint,
  formatting, and production build gates run, all pass or a pre-existing
  failure is documented with evidence.

## Assumptions

- Technical: Mission Control is a React/TypeScript page using the shared
  transport implementation and semantic CSS tokens (source:
  `apps/ui/src/features/mission-control/MissionControlPage.tsx`,
  `apps/ui/ARCHITECTURE.md`, `apps/ui/src/styles/theme.css`).
- Technical: the gateway already has one WebSocket connection and a scoped
  subscription manager suitable for an explicit dashboard stream (source:
  `gateway/gateway-ws-protocol/src/messages.rs`,
  `gateway/src/websocket/subscriptions.rs`).
- Technical: bounded session summaries and selected-session tokens/current plan
  are already available from execution state (source:
  `services/execution-state/src/service.rs`,
  `services/execution-state/src/types.rs`).
- Product: the approved direction is the Attention Radar mockup and visual
  language, initially for Mission Control only (source: user confirmation
  2026-07-15).
- Product: polling does not scale suitably as session count grows; a live
  dashboard stream should preserve observability without broadcasting raw
  traces (source: user direction 2026-07-15).
- Process: active feature contracts live in `docs/specs/`, contracts are
  bidirectionally linked under `contracts/<type>/`, and non-trivial interface
  work uses the full implementation loop (source: `docs/CONVENTIONS.md`,
  `.codex/skills/work-loop/SKILL.md`).
