# Spec: Surface Timeline Interleave (research page)

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, light mode)
- **Constrained by:** none
- **Brief:** UI regression report — surfaces must render inline with the turn that produced them
- **Contract:** none
- **Shape:** ui

## Objective

Research-page surfaces render after ALL turns (flat list at the bottom of
the timeline), so in multi-turn sessions a surface from the first answer
sits far below later exchanges. Expected: `[User] → [Agent response +
surface] → [User] → …`. The placement was never interleaved in code; it
only looked correct in single-exchange sessions (recent sessions are
multi-turn, exposing it).

The association data exists end-to-end but is dropped at the REST edge:
`session_surfaces.execution_id` is persisted but
`list_saved_session_surfaces` returns bare `WorkSurface`s; live
`surface_created` events DO carry `execution_id` but the UI discards it.

## Acceptance criteria

- [x] AC1 — REST `GET /api/sessions/:id/surfaces` returns
  `{execution_id, surface}` pairs (execution_id from the persisted record).
- [x] AC2 — UI keeps surfaces as (executionId, surface) pairs: live events
  attach the event's `execution_id`; snapshot load uses the REST pairs.
- [x] AC3 — MainColumn interleaves: after each turn block, render that
  turn's surfaces (match `executionId === turn.id`); surfaces with no
  matching turn (null id, or turn pruned) render after the last turn.
- [x] AC4 — Tests: endpoint pair-shape test, session-hook pair-state test,
  ResearchPage interleave test (surface under its turn, orphan at end).
- [x] AC5 — Live browser verification: reload a multi-turn session with
  surfaces and observe each surface under its producing turn.

## Boundaries

### Never do

- No changes to surface persistence, validation, events, or the renderer
  component itself — placement only.
- No new prompt text, no tool names anywhere.

## Testing strategy

Rust: surfaces endpoint tests. UI: vitest for hook + page. Gates: cargo
test (gateway), vitest run, `npm run build` (dist must be rebuilt — the
served bundle), then browser check via the running daemon.
