# Plan: Autonomy Ledger

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

## Approach

Add a small zbot-owned operational store under `zbot-conversation`, then expose
it through the gateway and Mission Control. A dedicated, typed ledger-resume
path—not intent text, generic metadata, or a caller packet—carries a server
constructed packet into the existing executor's transient system context.
Existing execution remains the only executor. The first release is manually
initiated and exposes timer eligibility only as observable state.

## Constraints

- No new dependency, top-level crate, worker, model call, or event transport.
- Engram is not the ledger backend; its facts and graph entities may be linked
  as evidence later through stable references.
- No automatic write-capable execution, semantic auto-attachment, cron wiring,
  or retry loop.
- `POST /api/autonomy/:id/resume` is an explicit user-selection boundary. It
  must load its own item and source session, verify `approved`, and never trust
  a packet, source agent, or item id from model output or generic metadata.

## Construction tests

**Integration tests:** SQLite schema/store round trip, strict gateway request
validation, approved-only server-built resume packet handoff, Mission Control
detail/actions, and a no-side-effect eligibility matrix.

**Manual verification:** in two separate browser sessions, create/approve an
item in one, select Resume in Mission Control in the other, inspect its
reference-only evidence and close it. Confirm the resulting session is new and
a similar ordinary request receives no ledger context.

## Design (LLD)

### Design decisions

The ledger is a narrow runtime record, not a general goal framework. Its stable
states are `proposed`, `approved`, `blocked`, `complete`, and `stale`; resume
is an execution request, not a persistent state. A user or deterministic policy
owns transitions. Traces to: AC1-AC3, AC6.

### Data & schema

`autonomy_items`, `autonomy_evidence`, and `autonomy_runs` live in
`conversations.db`. Items store a normalized title, objective, next action,
state, approval policy, source session, bounded evidence references, and
timestamps. Evidence rows link to sessions, artifacts, and future semantic ids
without copying their payloads. `autonomy_runs` adds a fixed
`resume_requested` audit outcome before an execution can start. Traces to:
AC1, AC2, AC4.

`LedgerResumePacket` is a typed, immutable DTO constructed only from an
already-loaded `approved` item and its reference-only evidence. It contains
`item_id`, `title` (<= 200 UTF-8 bytes), `objective` (<= 2,048 bytes),
`next_action` (<= 1,024 bytes), and <= 8 evidence entries of `kind` (<= 64
bytes) plus `reference_id` (<= 256 bytes). Its serialized form is <= 8 KiB.
Labels and all source transcript/evidence payload are excluded. Invalid UTF-8
sizes, missing rows, serialization errors, and any non-approved state fail
before execution and emit only a redacted diagnostic. Traces to: AC4.
Packet fields are opaque outside `zbot-conversation` and packet JSON escapes
tag delimiters before its system-context wrapper is rendered. Lifecycle audit
outcomes are trimmed, non-blank when supplied, and capped at 512 UTF-8 bytes.

### Interfaces & contracts

Gateway HTTP endpoints list/read/create/update ledger items, provide an
explicit `POST /api/autonomy/:id/resume`, and provide
`GET /api/autonomy/:id/eligibility?trigger=timer`. Resume accepts no packet
body: the handler parameter-loads the exact item/evidence, verifies the state,
resolves the source session's root agent, writes the audit record, and calls a
ledger-specific runtime method. The runtime starts a *new* session and passes
the packet through a dedicated `ExecutionConfig` field into transient system
instructions. The generic session-resume endpoint and persisted handle are
not reused. Strict request DTOs deny unknown fields and retain current field
bounds. Traces to: AC4, AC5, AC6.

### Component / module decomposition

`zbot-conversation` owns domain/store/schema and packet validation. Gateway
owns HTTP handlers and the sole trusted resume handoff; `gateway-execution`
renders the typed packet as non-persisted system context. Mission Control owns
a compact Decision Threads panel. Traces to: AC1-AC5.

### State & control flow

An explicit proposal is persisted as `proposed`; user approval transitions it
to `approved`; a terminal execution can record `blocked` or `complete`.
Ordinary requests have no ledger resolution or packet path. A click on one
approved item calls the resume endpoint; that endpoint builds the packet and
starts a new session. Review only reads the item. Traces to: AC2-AC5.

### Failure, edge cases & resilience

No match, multiple match, invalid transition, missing source session, packet
bound/serialization failure, or non-approved item fails closed. In every such
case the normal request path remains unchanged, no packet is injected, no
executor state is mutated, and no retry is attempted. Eligibility is a query,
not a trigger: it creates no run and performs no executor, tool, filesystem,
or external-service action. Traces to: AC3, AC4, AC6.

## Tasks

### T1: Durable ledger lifecycle store

**Depends on:** none

**Touches:** `stores/zbot-conversation/src/{domain,schema,lib}.rs`, new ledger
store module and tests

**Tests:**
- TDD: schema creates all three tables and indexes (AC1).
- TDD: valid transitions persist history; invalid transitions fail (AC2).
- TDD: evidence remains references rather than copied conversation payloads
  (AC1).

**Approach:**
- Define domain records, transition rules, store trait, SQLite implementation,
  and migration-safe schema setup.

**Done when:** focused `zbot-conversation` tests prove lifecycle persistence.

**Progress:** Complete. SQLite lifecycle tables, reference-only evidence, and
append-only audit runs are covered by focused tests.

### T2: Gateway ledger service and HTTP surface

**Depends on:** T1

**Touches:** `gateway/src/state/`, `gateway/src/http/`, gateway tests

**Tests:**
- TDD: create/list/detail/transition endpoints reject invalid input and expose
  only requested item data (AC1, AC2, AC5).
- Goal-based: existing session/execution endpoint contracts remain unchanged
  (AC7).

**Approach:**
- Wire the conversation store into gateway state and expose narrowly scoped
  ledger endpoints.

**Done when:** API tests exercise the item lifecycle without an executor change.

**Progress:** Complete. `AppState` wires one conversation-db-backed store and
the lifecycle endpoints are covered by gateway integration tests.

### T3: Server-built bounded resume context

**Depends on:** T1, T2

**Touches:** `stores/zbot-conversation/src/{domain,autonomy,lib}.rs`,
`gateway/src/{http/autonomy.rs,services/runtime.rs}`,
`gateway/gateway-execution/src/{config.rs,runner/invoke_bootstrap.rs}`, gateway
and gateway-execution tests

**Tests:**
- TDD: the ordinary execution configuration contains no ledger packet, and no
  model output or generic request metadata can select an item id (AC3).
- TDD: explicit resume of one approved item yields exactly one immutable,
  reference-only packet; duplicate, non-approved, missing-source, unknown,
  oversized, and SQL-shaped inputs cause no injection (AC4).
- Goal-based: an audit/store/serialization failure occurs before runtime
  invocation, creates no executor state, and has no retry (AC4).
- Goal-based: source transcript and evidence labels/payload are absent from
  rendered system context (AC4).

**Approach:**
- Add the typed `LedgerResumePacket` and a ledger-specific runtime entry point.
  The HTTP handler is the trust boundary: it parameter-loads the URL-selected
  item/evidence, rechecks `approved`, records `resume_requested`, resolves the
  source session root agent, and creates a new execution. `ExecutionConfig`
  gets one additive typed field; invoke bootstrap renders it as delimited,
  non-persisted system data with an instruction that packet values are data,
  never instructions. Do not change generic resume, plan middleware, model
  output, or arbitrary caller messages.

**Done when:** only a user-selected approved item can reach one new executor
session with a bounded packet, and all other paths leave execution unchanged.

**Progress:** Complete. The store atomically rechecks approval, bounds and
serializes a reference-only packet, and writes `resume_requested` before the
ledger-specific runtime entry point creates a fresh session. Ordinary execution
configuration has no packet path; focused store, gateway, and execution tests
cover the fail-closed cases.

### T4: Mission Control Decision Threads detail and resume surface

**Depends on:** T2, T3

**Touches:** `apps/ui/src/features/mission-control/`, transport types/client,
UI tests

**Tests:**
- Visual/integration: an item shows its state, evidence count/detail, source
  session, and next action and exposes valid lifecycle transitions (AC5).
- Visual/integration: Resume is available only for an approved item with a
  source session, calls only its item endpoint, and reports its new session
  result without touching another item/session (AC5).

**Approach:**
- Extend the existing compact panel and detail inspector using existing Mission
  Control transport conventions. Review remains a read-only detail fetch;
  Resume is its own explicit action and never reuses the generic session-resume
  control.

**Done when:** a user can inspect evidence and explicitly start one approved
item in a new session from Mission Control.

**Progress:** Complete. The Decision Threads panel fetches evidence detail on
explicit inspection and exposes Resume only for an approved item with a source
session; its transport calls only that item-specific resume endpoint.

### T5: Read-only eligibility, gates, and manual cross-session verification

**Depends on:** T1-T4

**Touches:** ledger service/tests, `docs/specs/autonomy-ledger/`

**Tests:**
- TDD: `GET .../eligibility?trigger=timer` applies this fixed projection with
  no stateful side effect: every `proposed`, `blocked`, `complete`, and `stale`
  item is ineligible; `approved/manual` is ineligible
  (`manual_resume_required`); `approved/ask_once` and
  `approved/auto_readonly` are eligible for a future non-writing timer
  integration (AC6).
- TDD: all eligibility reads leave timestamps/state/runs unchanged and invoke
  no runner/session spawn/tool/external/filesystem operation (AC6).
- Goal-based: workspace lint, typecheck, and focused test suites pass (AC7).
- Visual/manual QA: two-session create, approve, explicit resume, and close
  journey (AC3-AC5).

**Approach:**
- Implement the pure eligibility projection only; do not add schedule columns,
  cron hooks, workers, enqueueing, execution, or retries. Run gates and record
  the manual QA journey.

**Done when:** the feature is usable and its eligibility query cannot start,
enqueue, mutate, or retry an execution.

**Progress:** Complete. The eligibility endpoint is a pure timer-policy
projection with no scheduler/cron wiring. Gateway tests assert it preserves
item timestamps and audit-run count; no execution path is reachable from the
endpoint.

## Rollout

- **Delivery:** additive schema and routes; existing sessions, traces, memory,
  and executor behavior remain unchanged.
- **Deployment sequencing:** schema/store first, then gateway, context adapter,
  UI, and finally observable timer eligibility.

## Risks

- Intent relation semantics could introduce surprising context if resolution is
  too eager; default to `new` or `ambiguous`.
- Ledger context could grow into transcript replay; enforce a fixed packet
  structure and reference-only evidence.
- Operational state could be confused with memory; keep the store and API
  separate from Engram.

## Changelog

- 2026-07-09: initial implementation plan.
- 2026-07-15: completed the explicit approved-only resume path, Mission
  Control evidence/resume controls, and read-only timer eligibility projection.
