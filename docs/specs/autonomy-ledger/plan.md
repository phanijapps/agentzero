# Plan: Autonomy Ledger

- **Spec:** [`spec.md`](spec.md)
- **Status:** Executing

## Approach

Add a small zbot-owned operational store under `zbot-conversation`, then expose
it through the gateway and Mission Control. Existing intent analysis emits
relation metadata but cannot select or mutate a ledger item. Existing runtime
context middleware accepts a single resolved packet; existing execution remains
the only executor. The first release is manually initiated and uses timer
eligibility only as observable state.

## Constraints

- No new dependency, top-level crate, worker, model call, or event transport.
- Engram is not the ledger backend; its facts and graph entities may be linked
  as evidence later through stable references.
- No automatic write-capable execution or semantic auto-attachment.

## Construction tests

**Integration tests:** SQLite schema/store round trip, gateway HTTP contract,
Mission Control actions, and context-packet selection.

**Manual verification:** in two separate browser sessions, create/approve an
item in one, resume it by explicit title in the other, inspect evidence and
close it. Confirm a similar new request does not resume it automatically.

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
without copying their payloads. Traces to: AC1, AC2.

### Interfaces & contracts

Gateway HTTP endpoints list/read/create/update ledger items and produce a
single resume-context packet. The existing session/execution API is not
changed. Mission Control consumes the new endpoints. Traces to: AC4, AC5.

### Component / module decomposition

`zbot-conversation` owns domain/store/schema. Gateway owns service, HTTP
handlers, and intent/context adapters. Mission Control owns a compact Open
Loops panel. Traces to: AC1-AC5.

### State & control flow

An explicit proposal is persisted as `proposed`; user approval transitions it
to `approved`; a terminal execution can record `blocked` or `complete`. A
resolver returns `new`, `related`, `resume`, `review`, or `ambiguous`; only an
explicit `resume` with one item yields a context packet. Traces to: AC2-AC4.

### Failure, edge cases & resilience

No match, multiple match, invalid transition, missing evidence, and unapproved
timer eligibility fail closed. The normal request path proceeds without a ledger
context when resolution is uncertain. Traces to: AC3, AC4, AC6.

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

### T3: Intent relation and bounded resume context

**Depends on:** T1, T2

**Touches:** `gateway/gateway-execution/src/middleware/intent_analysis.rs`,
`runtime/agent-runtime/src/middleware/plan_block.rs`, gateway execution tests

**Tests:**
- TDD: similar requests return `related`/`ambiguous` and no item id (AC3).
- TDD: explicit unique resume yields exactly one bounded context packet (AC4).
- TDD: uncertain lookup leaves normal execution context unchanged (AC3, AC4).

**Approach:**
- Extend existing structured intent output with relation semantics and add a
  deterministic resolver/context adapter; do not add an LLM call.

**Done when:** the executor can receive one approved item's packet without
conversation replay or silent attachment.

**Progress:** Pending. This must use a trusted execution-context handoff; it
must not be implemented by concatenating an item into a caller-controlled
message or by allowing the intent model to choose an item id.

### T4: Mission Control Open Loops surface

**Depends on:** T2

**Touches:** `apps/ui/src/features/mission-control/`, transport types/client,
UI tests

**Tests:**
- Visual/integration: an item shows its state, evidence count, and next action
  and exposes approved transitions (AC5).
- Visual/integration: no unrelated session is changed by an item action (AC5).

**Approach:**
- Add a compact panel and detail inspector using existing Mission Control
  layout and transport conventions.

**Done when:** a user can inspect and transition one item from Mission Control.

**Progress:** Complete for inspection and explicit lifecycle transitions. The
first panel intentionally does not provide a resume control until T3 provides
the trusted context handoff.

### T5: Eligibility, tests, and manual cross-session verification

**Depends on:** T1-T4

**Touches:** ledger service/tests, `docs/specs/autonomy-ledger/`

**Tests:**
- TDD: unapproved items are ineligible; approved timer items are observable but
  do not execute (AC6).
- Goal-based: workspace lint, typecheck, and focused test suites pass (AC7).
- Visual/manual QA: two-session create, approve, explicit resume, and close
  journey (AC3-AC5).

**Approach:**
- Implement eligibility calculation only, run gates, and record manual QA.

**Done when:** the feature is usable and cannot autonomously write or execute.

**Progress:** Pending after T3. No trigger or executor wiring has been added,
so unapproved or approved items cannot run autonomously.

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
