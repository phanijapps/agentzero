# Spec: Mission Control Performance

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Make `/mission-control` and the Research session views load from bounded or
scoped data instead of dragging full session, execution, and log payloads
through the browser on initial render. A user with a growing `conversations.db`
should see Mission Control and `/research/:sessionId` populate from small
summary/detail requests, while full logs and tool traces are fetched only for
the selected session. The first pass optimizes query shape and data volume;
destructive cleanup or archival is not the primary performance mechanism.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Keep the initial Mission Control list bounded to a small default page, with a
  hard server-side cap.
- Keep the Research drawer bounded to a small recent-session page, and keep
  `/research/:sessionId` scoped to that specific session plus its child
  executions.
- Preserve the existing Mission Control route and user workflow: list on the
  left, selected-session detail on the right.
- Fetch full logs and tool traces lazily for the selected session only.
- Keep status and token totals sourced from the execution-state tables, not
  inferred from execution log rows.

### Ask first

- Changing session retention, archival, deletion, or log cleanup policy.
- Replacing the existing `/api/logs/sessions/:id` detail contract instead of
  adding a lighter summary path.
- Adding new runtime dependencies, background workers, or a new top-level
  service boundary.

### Never do

- Never make page load fast by deleting, hiding, or truncating persisted user
  records.
- Never load all sessions, all executions, all messages, or all execution logs
  on initial Mission Control render.
- Never load all sessions, all executions, all messages, or all execution logs
  to open a specific Research session route.
- Never introduce another client-side full-list join to replace the current
  `listSessionsFull({ limit: 200 })` behavior.
- Never break deep inspection of a selected session's messages, tool calls, or
  subagent trace.

## Testing Strategy

- Summary API behavior: **TDD**. Repository and handler tests should prove the
  endpoint applies default limits, clamps excessive limits, returns root-session
  summaries, carries root execution IDs, and avoids per-session execution
  queries.
- Mission Control data flow: **TDD**. UI hook/page tests should prove initial
  render calls the summary endpoint and does not call
  `/api/executions/v2/sessions/full`.
- Research data flow: **TDD**. UI hook tests should prove the Research drawer
  uses the bounded summary endpoint and `/research/:sessionId` snapshots use
  scoped session detail calls, not a global log-session list.
- Selected-session detail loading: **TDD plus goal-based check**. Component/hook
  tests should prove messages/tools share a selected-session detail fetch where
  practical, and build/typecheck should prove existing detail panes still
  compile.
- Runtime smoke: **goal-based check**. Run targeted Rust and UI test commands,
  plus lint/typecheck where practical, to verify the changed API and UI surfaces.

## Acceptance Criteria

- [x] Mission Control initial render uses a dedicated bounded summary endpoint
  with default limit 50 and server-side max 200.
- [x] Mission Control no longer calls `listSessionsFull({ limit: 200 })` during
  initial page load.
- [x] The summary endpoint returns only list/header fields, root execution ID,
  aggregate token totals, subagent count, and mode; it does not return execution
  arrays, child execution ID arrays, full logs, messages, checkpoints, or tool
  result payloads.
- [x] The backend summary query fetches executions for the visible page in one
  batched query rather than one query per session.
- [x] Full logs/tool trace data are fetched only after a session is selected,
  and selected-session detail is not fetched twice by sibling panes on the same
  render.
- [x] Existing Mission Control list filtering, KPI display, token display,
  selected-session messages, and tools trace remain functional.
- [x] Research drawer loading uses the bounded summary endpoint instead of an
  unbounded log-session scan.
- [x] Opening `/research/:sessionId` snapshots that session via scoped detail
  calls and does not scan every log session to find matching rows.
- [x] Research and app startup avoid duplicate dev-mode StrictMode request
  bursts for health checks, recent sessions, drawer sessions, and direct
  session hydration.
- [x] Mission Control avoids duplicate dev-mode StrictMode request bursts for
  the session list and selected-session detail loading.
- [x] Mission Control loads per-execution token slices only for the selected
  session, not for every row in the initial list payload.
- [x] Targeted Rust tests, targeted UI tests, and formatting/lint/typecheck
  gates for touched areas pass or are explicitly documented with a pre-existing
  blocker.

## Assumptions

- Technical: Mission Control is React/Vite UI backed by Rust/Axum services
  (source: `apps/ui/package.json`; `services/execution-state/Cargo.toml`;
  `services/api-logs/Cargo.toml`).
- Technical: Mission Control currently loads log sessions with `root_only:
  true, limit: 200` and also calls the v2 full-sessions endpoint for
  token/status data (source:
  `apps/ui/src/features/mission-control/MissionControlPage.tsx`).
- Technical: `/api/executions/v2/sessions/full` lists sessions, then loads
  executions per session, so it has N+1 query behavior (source:
  `services/execution-state/src/repository.rs`).
- Technical: `/api/logs/sessions` aggregates over `execution_logs` before
  ordering/limiting, so database growth affects the page even with a UI limit
  (source: `services/api-logs/src/repository.rs`).
- Process: no `docs/CONVENTIONS.md` or `docs/CHARTER.md` exists in this
  workspace (source: `test -f docs/CONVENTIONS.md; test -f docs/CHARTER.md`,
  both missing on 2026-06-09).
- Process: active specs are listed in `docs/specs/README.md` (source:
  repository read 2026-06-09).
- Product: the right target is "Mission Control should load a bounded
  lightweight summary first, then lazy-load selected-session detail," not
  "delete old records" or "raise global limits" (source: user confirmation
  2026-06-09).
- Product: a good first implementation target is 50 summary rows by default,
  with pagination/cursor support, while KPIs come from aggregate endpoints
  rather than the visible row set (source: user confirmation 2026-06-09).
- Product: exact millisecond SLOs are less important for this pass than removing
  unbounded/N+1/full-detail loads from initial render (source: user confirmation
  2026-06-09).
