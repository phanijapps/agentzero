# Spec: Research Live Artifact Refresh

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** ui

> **Mode:** light (no risk trigger fired).

## Objective

When a newly created Research session completes with a persisted artifact, its
artifact chip appears in the open Research view without requiring a browser
refresh, while Research continues to use event-driven snapshots rather than
interval polling.

## Boundaries

### Always do

- Use the server-emitted session identifier to target the existing snapshot refresh.
- Preserve the current REST artifact manifest and WebSocket transport contracts.
- Add a regression test covering the fresh-session completion race.

### Ask first

- Reintroducing polling or changing gateway event/API contracts.
- Broadening this fix to session-subscription lifecycle cleanup.

### Never do

- Add a dependency, module boundary, or new top-level directory.
- Change artifact persistence, serving, or the gateway solely for this UI defect.

## Testing Strategy

- **TDD:** a focused Research hook integration test reproduces a freshly bound
  session completing before React re-renders; it verifies that the artifact
  manifest reaches Research state without reload.
- **Goal-based check:** TypeScript/lint/build checks confirm the UI still
  compiles without transport-contract changes.
- **Visual/manual QA:** complete a fast Research request that declares an
  artifact and confirm its chip appears before refreshing the page.

## Acceptance Criteria

- [x] Given a newly created Research session, when the server binds its ID and
  the root completion event arrives immediately with a persisted artifact, the
  open page shows the artifact without a browser refresh.
- [x] Root completion refreshes only the active session named by the trusted
  server event and does not replace another selected Research session.
- [x] Research uses no interval-based artifact polling.

## Assumptions

- Technical: Research is a React 19 + TypeScript + Vitest UI feature using the
  existing transport abstraction (source: `apps/ui/AGENTS.md`).
- Technical: the existing `agent_completed` event and `/artifacts` manifest are
  the intended live-refresh inputs (source: `apps/ui/src/features/research-v2/useResearchSession.ts`).
- Product: an artifact from a completed Research session should appear without
  manual refresh (source: user confirmation 2026-07-15).
- Process: the implementation is a focused UI defect correction with no new
  interface or dependency (source: user direction 2026-07-15).
