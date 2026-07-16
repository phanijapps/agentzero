# Spec: Research Session Switching

Mode: light (no risk trigger fired)

- **Status:** Shipped
- **Owner:** phanijapps

## Objective

Selecting a completed Research session while another Research session is open
must keep the selected URL and render the selected session, rather than
redirecting back to stale in-memory session state.

## Acceptance Criteria

- [x] Given completed sessions A and B, when the user selects B from A, the
  route remains `/research/B` and the rendered session state is B.
- [x] URL synchronization still navigates a new, server-bound Research session
  from `/research` to its durable `/research/<session-id>` route.
- [x] The focused hook regression test, UI lint, and UI production build pass.

## Tasks

1. Add a red hook-level route-switch regression test using two completed
   snapshots.
2. Restrict state-to-URL synchronization to flows with no route-selected
   session, then make the test green.
3. Run focused UI gates and review the minimal diff.

## Verification

TDD: the hook tests cover both response orders: a selected B snapshot that is
slow to resolve and an obsolete A snapshot that resolves after B. Both keep B
selected and rendered. The final checks passed on 2026-07-14:

- 57 Research UI tests (`useResearchSession`, `ResearchPage`, and
  `session-snapshot`)
- UI lint (0 errors; existing repository warnings only)
- UI production build
- A headless browser switch from an existing IBM session to
  `sess-f418ee1d-4941-4ce2-b0fb-9bd6faedfa55`

## Scope

Touches only `apps/ui/src/features/research-v2/useResearchSession.ts`, its
focused test, and this spec. It does not alter session persistence, gateway
routes, API contracts, or the session drawer.

Tempted to add navigation state or a new routing abstraction; declining — the
existing URL is already authoritative for a user-initiated session selection.
