# Spec: Persistent work surfaces

- **Status:** Shipped (2026-07-28)
- **Owner:** zbot maintainers
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** [`contracts/openapi/work-surfaces.yaml`](../../../contracts/openapi/work-surfaces.yaml), [`contracts/asyncapi/agent-surfaces.yaml`](../../../contracts/asyncapi/agent-surfaces.yaml)
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Let a user opt in from Settings to retaining validated A2UI work surfaces across
page reloads and daemon restarts. When enabled, Quick Chat and Research restore
the latest bounded set of surfaces for the selected session and continue to
replace updates by stable `surface_id`; when disabled, live surfaces continue
to work but no surface is saved or restored. Turning the setting off retains
previously saved surfaces, and a separate explicit action permanently clears
all saved infographics.

## Boundaries

### Always do

- Validate every surface against `zbot/work-surface/v1` before writing it and
  after deserializing it for a client; invalid rows fail closed.
- Bound persisted descriptors to the existing catalog payload limit and retain
  at most 16 surfaces per session.
- Apply settings changes live without requiring a daemon restart, and preserve
  session-scoped rows until the session or the explicit global clear action is
  deleted.
- Treat the configured gateway reachability as a single-owner trust boundary:
  `session_id` is correlation data, not an authorization credential, and a
  browser `Origin` must match `Host` before any persistence read or mutation.

### Ask first

- Changing the opt-in default, the 16-surface retention limit, or the
  retain-on-disable behavior.
- Persisting surfaces for a client other than Quick Chat and Research.
- Adding retention schedules, export/import, synchronization, or per-surface
  management.

### Never do

- Never persist a rejected, malformed, oversized, unknown-catalog, or
  executable/actionable descriptor through this feature.
- Never persist or restore `ApprovalGate` or a component outside the explicit
  display-only persistence allowlist, even when that component is permitted for
  a transient live surface.
- Never restore saved surfaces while persistence is disabled, and never delete
  saved rows merely because the toggle was switched off.
- Never add a new top-level crate/package, persistence backend, renderer path,
  state-management layer, or runtime dependency.
- Never interpolate a session ID, surface ID, execution ID, table name, or
  descriptor value into SQL.

## Testing Strategy

- SQLite migration, stable-ID upsert/delete, per-session retention, session
  cascade, and global clear: **TDD**, because these are deterministic storage
  invariants.
- Configuration default, durable update, and live runtime gating:
  **TDD plus goal-based integration**, because the file-backed setting and
  execution-state mirror must agree without restart.
- REST response shape, disabled behavior, validation-on-read, and identifier
  encoding: **TDD**, exercised at handler and transport boundaries.
- Quick Chat and Research hydration plus stable-ID live replacement:
  **TDD hook/integration tests**, because reload behavior spans transport and
  React state.
- Toggle and destructive clear interaction: **visual/manual QA backed by
  component tests** that assert the labels, disabled/loading states,
  confirmation, and visible success/error feedback.

Covered in PLAN stubs: AC1-AC9. Uncovered: none.

## Acceptance Criteria

- [x] Given a fresh or upgraded vault, the database contains a
  `session_surfaces` table keyed by `(session_id, surface_id)` with session
  cascade deletion, and existing session/message data remains intact.
- [x] Given persistence is enabled, when a validated surface is created or
  updated, the latest descriptor is durably upserted by stable `surface_id`;
  a delete event removes it, and no session retains more than 16 descriptors.
- [x] Given persistence is disabled, live create/update/delete events still
  reach subscribed clients, but no new descriptor is stored and session
  restore returns an empty list without deleting older saved rows.
- [x] Given saved surfaces exist, when Quick Chat reloads or Research opens
  that session while persistence is enabled, the valid descriptors render
  once and later live updates replace matching stable IDs without duplication.
- [x] Given a stored row is malformed, oversized, or fails the current catalog,
  when it is read, it is omitted from the response and neither its descriptor
  data nor bound values are emitted in logs or errors.
- [x] Given the Settings UI, the user can enable or disable “Persist
  infographics” without a restart and sees explanatory text that disabling
  retains saved data.
- [x] Given saved infographics exist, the separate “Clear saved infographics”
  action requires a fixed server-verified confirmation value, deletes all
  stored surface rows, reports the number cleared, and does not alter messages,
  sessions, artifacts, memory, or currently displayed live surfaces.
- [x] Given a browser request to any persistence/settings endpoint, `Origin`
  must match `Host` before request-body extraction or any read/write/delete;
  an absent `Origin` remains valid for native clients within the configured
  local/LAN single-owner boundary, and `session_id` is never treated as
  authentication or multi-user authorization.
- [x] Given any `session_surfaces` query or mutation, every data value is bound
  through rusqlite parameters inside the appropriate transaction, the clear
  operation targets only the fixed `session_surfaces` table, and no
  caller/model-controlled value is used as SQL syntax.

## Assumptions

- Technical: app settings already persist in `settings.json`, and execution
  settings expose UI-facing feature configuration (source:
  `gateway/gateway-services/src/settings.rs`).
- Technical: session history is SQLite-backed while Quick Chat and Research
  currently hold surfaces only in React state (source:
  `docs/architecture/architecture.md`,
  `apps/ui/src/features/chat-v2/useQuickChat.ts`,
  `apps/ui/src/features/research-v2/useResearchSession.ts`).
- Technical: the established stack is Axum, rusqlite, React 19, TypeScript,
  Vitest, and the UI transport abstraction (source:
  `docs/architecture/architecture.md`, `apps/ui/ARCHITECTURE.md`).
- Process: mixed UI/service/data changes require a living spec, explicit
  acceptance criteria, and contract traceability (source:
  `docs/CONVENTIONS.md §4`).
- Product: persistence defaults off, covers Quick Chat and Research, and does
  not disable live infographics (source: user confirmation 2026-07-28).
- Product: disabling persistence retains saved rows; only the separate clear
  action deletes them (source: user confirmation 2026-07-28).
