# Plan: SQLite Runtime Store Split

- **Spec:** [`spec.md`](spec.md)
- **Status:** Complete

## Approach

Extract the `DatabaseManager`-backed conversation/execution runtime surface
first, preserving its public behavior and on-disk location. Then redirect
gateway/runtime imports to that crate, quarantine the remaining semantic
SQLite exports behind a legacy crate boundary, and prove the active composition
uses Engram through `zbot-engram-adapter`. No schemas or user data paths move.

## Constraints

- Preserve `~/Documents/zbot/conversations.db` exactly.
- Follow the Engram provider façade integration documented in
  `/home/videogamer/projects/mem-alpha/docs/guides/how-to/integrate-engram-as-library.md`.
- Do not introduce a new persistence backend or direct gateway access to
  Engram SQLite internals.

## Construction tests

- Integration: start runtime state/log services against a pre-existing
  `conversations.db`, then reopen it and verify the session remains visible.
- Goal-based: production gateway/runtime imports contain no legacy semantic
  SQLite crate references after the redirect.

## Design (LLD)

### Component decomposition

`zbot-runtime-sqlite` owns `DatabaseManager`, runtime schema, and repositories
for sessions, executions, logs, checkpoints, distillation status, and outbox.
`zbot-stores-sqlite-legacy` owns historical semantic SQLite modules only.
`zbot-engram-adapter` remains the sole active semantic compatibility boundary.

### Data and schema

The runtime crate retains the existing `conversations.db` schema and path.
The legacy crate retains its current knowledge database schema only for
migration/parity fixtures; it does not gain new production tables.

### Failure and resilience

The extraction is compile-gated before legacy exports are removed. Existing
database open failures retain their current error behavior; no fallback creates
or migrates user data silently.

## Tasks

### T1: Inventory and isolate runtime SQLite exports [complete]

**Depends on:** none

**Tests:** TDD — compile a runtime-only consumer against the extracted public
surface; goal-based import inventory check.

**Approach:** Classify every `zbot-stores-sqlite` export as runtime or semantic;
extract `DatabaseManager`, runtime schema, and runtime repositories into
`stores/zbot-runtime-sqlite` without changing their database path behavior.

**Result:** `zbot-runtime-sqlite` now owns the compiled `DatabaseManager`
surface and runtime schema; legacy storage re-exports it while consumers are
redirected incrementally.

### T2: Redirect active composition to runtime SQLite [complete]

**Depends on:** T1

**Tests:** TDD — existing session/execution/log tests compile unchanged through
the new crate; integration test reopens the same `conversations.db`.

**Approach:** Update gateway, execution, daemon, CLI, and service dependencies
to use `zbot-runtime-sqlite`; preserve all runtime repository APIs where a
rename would add migration risk.

**Result:** Gateway shell, bridge, and execution runtime consumers now import
`zbot_runtime_sqlite::DatabaseManager`; a production-source scan finds no
remaining `zbot_stores_sqlite::DatabaseManager` imports in those packages.

### T3: Quarantine legacy semantic SQLite [complete]

**Depends on:** T2

**Tests:** Goal-based — production `gateway/`, `runtime/`, `services/`, and
`apps/` have no imports of the legacy semantic crate; migration/parity tests
remain runnable.

**Approach:** Rename or split the remaining semantic SQLite modules into
`zbot-stores-sqlite-legacy`, update fixture-only consumers, and constrain
production composition to `zbot-engram-adapter`.

**Result:** Active gateway/execution composition now carries only store traits
for goals, episodes, KG episodes, graph access, and compaction. Engram supplies
those trait objects; legacy SQLite imports remain only in parity/fixture tests.
Operational distillation-run tracking was moved to `zbot-runtime-sqlite`.

### T4: Document and verify the final boundary [complete]

**Depends on:** T3

**Tests:** Goal-based dependency scan and focused workspace tests; manual
fresh/existing data-directory daemon smoke.

**Approach:** Add dependency-boundary checks and update architecture/spec docs
with the runtime-vs-legacy ownership map and verification commands.

**Result:** The architecture ownership map documents the split. Focused runtime,
gateway, and execution checks/tests pass with `conversations.db` unchanged.

### T5: Physically relocate runtime SQLite sources

**Depends on:** T1, T2, T3

**Tests:** Goal-based — `zbot-runtime-sqlite` compiles with local runtime
modules only; no `#[path]` bridge points into `zbot-stores-sqlite`; focused
runtime/gateway/execution checks and tests remain green.

**Approach:** Move the runtime connection, schema, system-profile, and
distillation-run modules into `stores/zbot-runtime-sqlite/src`. Re-export the
system-profile module from the legacy crate only where its legacy knowledge DB
still requires it. Do not rename crates, change schemas, or move database files.

**Result:** The runtime connection, schema, system profile, and distillation
repository now live directly in `zbot-runtime-sqlite`; the legacy crate only
re-exports compatibility symbols used by legacy semantic code.
