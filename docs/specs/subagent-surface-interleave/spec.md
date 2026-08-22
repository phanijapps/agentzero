# Spec: Subagent-created surfaces interleave under their root turn

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** none (single-phase, light mode with live gate)
- **Constrained by:** #257 (surface timeline interleave), #256 (routing freedom)
- **Brief:** user regression report — interleave broke again for ward-agent flows
- **Contract:** none
- **Shape:** ui

## Objective

#257 interleaved surfaces by matching `execution_id` against TOP-LEVEL
turns only. Subagent turns are nested (`turn.subagents[]`), and since #256
restored routing freedom, ward-agents (not root) build and present — so
subagent-created surfaces never matched any top-level turn and orphaned at
the bottom of the chat. Additionally, snapshot subagent turns are keyed by
session id while surfaces carry execution ids — the two could never match.

## Acceptance criteria

- [x] AC1 — REST surfaces response carries `session_id` alongside
  `execution_id` (the creating agent's session; ward-agent surfaces carry
  the child session id).
- [x] AC2 — UI keeps both keys from live events and snapshot; ownership
  maps top-level turns by execution id AND nested subagent turns by their
  id (session id in snapshot, execution id in live), mapping each surface
  to its subagent's PARENT root turn.
- [x] AC3 — Orphans (no owner) still render after the last turn.
- [x] AC4 — Gates green; live verification: a fresh ward-agent flow's
  surfaces render beneath the producing turn (sess-b5425bd5: turn →
  plan-surface → comparison-surface in DOM order), and the ward-agent
  surface in sess-e5d11d51 attaches via the session-id key.

## Testing strategy

Endpoint pair test (session_id), UI suites (1355), dist rebuilt, live
browser DOM-order verification on fresh and historical sessions.
