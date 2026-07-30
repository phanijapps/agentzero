# Plan: Persistent work surfaces

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog.

## Approach

Add a versioned `session_surfaces` table to the existing runtime SQLite schema
and repository methods that treat the validated descriptor JSON as an opaque,
bounded value. Add typed presentation settings whose persisted value is
mirrored into the shared execution-state service for hot event-path checks.
The existing gateway surface conversion remains the validation boundary:
accepted create/update/delete events update the table only when enabled.
Gateway-owned REST handlers validate saved descriptors again on read, return
them to the existing UI transport, and expose a separate global clear
operation. Quick Chat and Research fetch saved surfaces during their existing
bootstrap/snapshot flows.

## Constraints

- Preserve the supplementary, display-only behavior and validator contract in
  `docs/specs/automatic-work-surfaces/spec.md` and
  `contracts/asyncapi/agent-surfaces.yaml`.
- Reuse `StateService`, `SettingsService`, `ZbotWorkSurfaceCatalog`, the
  gateway `http::surfaces` module, and the existing UI transport.
- Keep model-owned descriptor content out of logs and error responses.
- Add no dependency, crate, renderer, or general-purpose persistence
  abstraction.

## Construction tests

- Integration: enable persistence, project a validated surface event, recreate
  the database/service and client hook, and verify the same stable descriptor
  restores once (AC2-AC4).
- Regression: repeat with persistence disabled and verify the live event still
  renders while REST restore is empty and pre-existing rows remain (AC3).
- Manual: toggle persistence, reload Quick Chat and a Research session, then
  confirm and clear saved infographics from Settings (AC4, AC6, AC7).

## Design (LLD)

### Design decisions

- Store descriptor JSON rather than a second component schema; the portable
  catalog remains the schema validator, followed by an explicit persistence
  allowlist that rejects `ApprovalGate` and any future actionable component.
- Mirror the durable setting into `StateService` as a live boolean so the event
  path and lifecycle reconciliation share one cheap gate.
- Keep clear explicit and global because the setting is application-wide and
  the user asked for one management action rather than per-session editing.

### Data & schema

Schema version 25 adds `session_surfaces(session_id, surface_id, execution_id,
surface_json, created_at, updated_at)` with a composite primary key and
`ON DELETE CASCADE` foreign keys. Upsert preserves `created_at`, replaces the
execution and descriptor, and prunes least-recently-updated rows beyond 16 for
that session.

### Interfaces & contracts

`GET /api/sessions/{sessionId}/surfaces` returns a bounded JSON array of
validated `WorkSurface` descriptors and returns an empty array while disabled.
`DELETE /api/surfaces/saved` clears all persisted surface rows and returns
`deletedCount` only when the request carries the fixed confirmation value.
`GET|PUT /api/settings/presentation` reads and updates
`{persistSurfaces: boolean}` with `restartRequired: false`. These operations are
defined in `contracts/openapi/work-surfaces.yaml`; live create/update/delete
events remain defined in `contracts/asyncapi/agent-surfaces.yaml`.

### Component / module decomposition

- `stores/zbot-runtime-sqlite/src/schema.rs`: migration and fresh schema.
- `services/execution-state/src/{types,repository,service}.rs`: opaque records,
  bounded CRUD, live enable flag, and cascade integration.
- `gateway/gateway-services/src/settings.rs`: typed durable presentation
  settings.
- `gateway/src/http/{settings,surfaces,mod}.rs`: configuration, restore, and
  clear handlers.
- `gateway/gateway-execution/src/{invoke/stream_event_processor,lifecycle}.rs`:
  persist accepted events and terminal plan updates behind the live flag.
- `apps/ui/src/services/transport/*`: typed REST methods.
- Quick Chat/Research hooks and Settings panel: hydration and controls.

### State & control flow

At boot, `AppState` loads the durable setting into `StateService`. A settings
update persists the file and updates the live flag. Accepted surface events are
broadcast regardless of the flag; when enabled, the same descriptor is
upserted before/alongside publication. Client bootstrap fetches saved surfaces,
then the existing stable-ID reducer handles subsequent WebSocket updates.

### Behavior & rules

- Default off; disabled reads return `[]` and disabled writes no-op.
- Toggle-off retains rows; re-enable makes them restorable again.
- Clear deletes only `session_surfaces`.
- Read validation skips invalid rows and logs only stable identifiers plus a
  generic reason code.
- Write and read paths reject `ApprovalGate` and any component outside the
  closed display-only persistence allowlist, even if the broader portable
  catalog accepts it for transient live use.
- Persistence endpoints use a shared same-origin check: when a browser sends
  `Origin`, it must equal the request `Host`; requests without `Origin` remain
  available to native clients inside the gateway's configured trust boundary.
- Every SQL data value is a rusqlite parameter; clear uses one fixed SQL
  statement and requires the literal confirmation value
  `clear_saved_infographics`.

### Failure, edge cases & resilience

Surface persistence is supplementary: database/settings failures warn with
bounded metadata and never block event publication or the canonical response.
Restore failure leaves the client with no saved surfaces while live streaming
continues. A stale hook response is ignored after a session switch/unmount.

### Quality attributes (NFRs)

Reads and writes are bounded to 16 descriptors per session and the existing
64-KiB descriptor limit. SQLite operations use parameters and transactions.
The UI controls expose pending, success, failure, and confirmation states with
semantic existing component classes.

### Dependencies & integration

No external dependency is added. The server and bundled UI ship together; an
older UI ignores the additive endpoints, and an older database migrates
forward on startup.

## Tasks

### T1: Schema migration and bounded stable-ID storage invariants are green

**Depends on:** none

**Touches:** `stores/zbot-runtime-sqlite/src/schema.rs`, `services/execution-state/src/{types,repository,service}.rs`

**Tests:**
- TDD: `session_surfaces_schema_migrates_from_v24_without_data_loss` asserts the
  composite key and cascade table exist after migration (AC1). `stub: true`
- TDD: `session_surfaces_fresh_schema_has_composite_key_cascade_and_v25`
  asserts fresh initialization, schema version, composite primary key, and
  session cascade (AC1). `stub: true`
- TDD: `session_surfaces_upsert_delete_and_prune_to_sixteen` asserts replacement,
  stable ordering, deletion, per-session retention, and parameterized values
  containing SQL metacharacters (AC2, AC9). `stub: true`
- TDD: `session_surface_clear_preserves_sessions_messages_artifacts_and_memory`
  seeds and asserts each named persisted store remains while only surface rows
  are removed (AC7). `stub: true`

**Approach:**
- Add the v25 migration and current-schema table/index.
- Add opaque snapshot CRUD and a live `AtomicBool` gate to `StateService`.
- Include surface rows in explicit session cascade paths.

**Done when:** focused runtime-schema and execution-state tests pass.

### T2: Typed live presentation setting persists without restart

**Depends on:** T1

**Touches:** `gateway/gateway-services/src/settings.rs`, `gateway/src/state/mod.rs`, `gateway/src/http/settings.rs`

**Tests:**
- TDD: Settings default to disabled, round-trip `persistSurfaces`, and preserve
  unrelated settings (AC3, AC6). `stub: true`
- TDD: Handler update changes the shared live gate and returns
  `restartRequired: false` (AC6). `stub: true`

**Approach:**
- Add `PresentationSettings` to `AppSettings` and focused service methods.
- Initialize/update the `StateService` gate from the durable value.

**Done when:** settings service and handler tests prove durable/live agreement.

### T3: Validated events persist and REST restore/clear fail closed

**Depends on:** T1, T2

**Touches:** `runtime/agent-surfaces/src/lib.rs`, `gateway/gateway-execution/src/{invoke/stream_event_processor,lifecycle}.rs`, `gateway/src/http/{surfaces,mod}.rs`, `contracts/openapi/work-surfaces.yaml`, `contracts/asyncapi/agent-surfaces.yaml`

**Tests:**
- TDD: Create/update/delete and terminal plan events mutate storage only when the
  live gate is enabled without affecting event publication (AC2, AC3).
  `stub: true`
- TDD: Persistence write/read rejects `ApprovalGate` and every component
  outside the explicit display-only allowlist (AC2, AC5). `stub: true`
- TDD: REST restore returns at most 16 valid descriptors, returns `[]` when
  disabled, and omits corrupt/invalid/actionable rows without exposing
  descriptor data (AC3, AC5). `stub: true`
- TDD: Same-origin checks run before body extraction or effects, while absent-Origin
  native requests remain compatible with the configured single-owner gateway
  boundary (AC8). `stub: true`
- TDD: Clear rejects a missing/incorrect confirmation value, returns the exact
  deleted count for the correct value, and touches no other tables (AC7).
  `stub: true`

**Approach:**
- Persist only already-validated gateway events; validate again in GET.
- Add bounded restore and clear handlers with generic errors.
- Persist lifecycle-reconciled plan surface updates through the same service
  invariant.

**Done when:** gateway-execution and handler tests cover enabled, disabled,
invalid, delete, and clear paths.

### T4: Quick Chat and Research restore saved surfaces once

**Depends on:** T3

**Touches:** `apps/ui/src/services/transport/{types,interface,http,index}.ts`, `apps/ui/src/features/chat-v2/useQuickChat.ts`, `apps/ui/src/features/research-v2/{session-snapshot,useResearchSession}.ts`

**Tests:**
- TDD: Transport encodes session IDs and parses restore, settings, and clear
  responses (AC3, AC4, AC6, AC7). `stub: true`
- TDD: Quick Chat bootstrap hydrates saved surfaces and live stable-ID updates
  replace rather than duplicate them (AC4). `stub: true`
- TDD: Research snapshot hydrates saved surfaces and ignores stale session results
  after switching (AC4). `stub: true`

**Approach:**
- Add typed transport methods.
- Fetch surfaces alongside existing bootstrap/snapshot calls.
- Set surface state only for the active session; retain existing event reducer.

**Done when:** scoped Vitest hook/transport tests pass.

### T5: Settings exposes opt-in persistence and explicit confirmed clear

**Depends on:** T4

**Touches:** `apps/ui/src/features/settings/WebSettingsPanel.tsx`, `apps/ui/src/features/settings/WebSettingsPanel.test.tsx`, existing settings styles only if needed

**Tests:**
- TDD component tests assert default/off state, save feedback, retain-on-disable
  explanation, clear confirmation, pending state, deleted count, and error
  feedback (AC6, AC7). `stub: true`
- Architecture/component coverage asserts clear only invokes the dedicated
  storage endpoint; surface state remains local to the active Quick Chat and
  Research hooks and receives no clear/reset dispatch (AC7). `stub: true`
- Visual/manual QA: enable, create an infographic, refresh Quick Chat and
  Research, disable/re-enable, then clear and reload; record completed checks
  and results in `docs/specs/persistent-work-surfaces/manual-qa.md` (AC3, AC4,
  AC6, AC7).

**Approach:**
- Add a Presentation section using existing toggle/button classes.
- Use an explicit confirmation before the destructive clear request.

**Done when:** component tests pass and the recorded manual flow matches all
visible states.

### T6: Integrated gates, security review, and living docs are clean

**Depends on:** T1-T5

**Touches:** `docs/specs/persistent-work-surfaces/*`, `docs/specs/README.md`

**Tests:**
- Goal-based: Rust fmt, focused Rust tests, workspace check/clippy, scoped UI
  tests/build, contract parse, and spec-status lint pass.
- Goal-based review: Security review covers endpoint access, SQL parameterization, model-output
  validation, bounded errors, and destructive clear scope.
- `stub: no stub (goal-based/review mode)`.

**Approach:**
- Run the full work-loop gates and specialist reviews.
- Mark acceptance criteria/status only from evidence.

**Done when:** gates are green and all required reviewers report clean.

## Rollout

Schema migration ships before code reads/writes the table in the same daemon
binary. The feature is dark by default and reversible by disabling the setting;
that leaves saved rows intact. Rollback to an older binary ignores the additive
table and settings field. Clear is intentionally irreversible for surface rows
only and requires explicit UI confirmation.

## Risks

- A model descriptor can contain sensitive answer data; opt-in, local storage,
  validation, bounded retention, and explicit clear reduce but do not eliminate
  local-at-rest exposure.
- Settings-file and live-gate divergence could store unexpectedly; boot and
  update tests must prove they change together.
- Hydration can race session switching or live WebSocket updates; hooks must
  ignore stale responses and merge by stable identifier.
- Clear is destructive; its SQL target and UI confirmation must remain narrow.

## Changelog

- 2026-07-28: initial plan; user confirmed opt-in persistence, live behavior
  while disabled, retain-on-disable, and separate explicit clear.
- 2026-07-28: implementation completed. Adversarial review found that a
  non-persistable update could leave an older same-ID descriptor saved; the
  event path now evicts that stale row. Adversarial re-review, security
  review, and quality review are clean.
