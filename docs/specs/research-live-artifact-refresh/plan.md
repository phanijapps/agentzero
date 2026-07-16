# Plan: Research Live Artifact Refresh

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Keep the event-driven R14f design. Make the root-completion callback use the
authoritative `session_id` carried by the WebSocket event, guard it against the
currently active Research session, and refresh through the existing snapshot
function. Pin the new-session race with a focused integration test.

Declined: restore the old five-second poll — it hides the stale-closure race and
adds recurring requests to every active Research session. Declined: gateway or
artifact-store changes — the artifact manifest is already persisted before the
completion event and the existing client endpoint supplies it.

## Tasks

### T1: Fresh Research completion updates artifacts without reload

**Depends on:** none

**Touches:** `apps/ui/src/features/research-v2/useResearchSession.ts`, `apps/ui/src/features/research-v2/useResearchSession.test.ts`

**Tests:**

- TDD: begin from an unscoped Research route, bind a server session through the
  live stream, immediately deliver root `agent_completed`, and assert the
  hydrated artifact is available without re-opening the route (AC1).
- TDD: deliver a completion for a non-active session and assert it cannot
  overwrite the selected session (AC2).

**Approach:**

- Carry the event's validated session ID into root-completion handling instead
  of reading the render-time `state.sessionId` closure.
- Track the active server-bound session in a ref so late events are ignored.
- Retain existing execution de-duplication only after a valid active-session
  match, then call `hydrateFromSnapshot`.

**Done when:** the new tests are green and an artifact completes into the live
Research state without a page reload.

## Rollout

Ship with the UI bundle; no migration, feature flag, API, or deployment
ordering is required. Rollback is reverting this focused UI change.

## Risks

An event from an old session could incorrectly hydrate the current route; the
active-session guard and regression test make that path fail closed.

## Changelog

- 2026-07-15: initial light-mode defect-fix plan.
- 2026-07-15: used the server-owned session ID for root-completion snapshots,
  added fresh-session and cross-session regression coverage, and repaired the
  Research transport test double for the existing session-state probe.
