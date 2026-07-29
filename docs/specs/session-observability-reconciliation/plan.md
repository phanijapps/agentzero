# Plan: Session Observability Reconciliation

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Correct the two historical assumptions exposed by continuation roots: log rows
are grouped by a physical execution and their status is inferred from log
timestamps. Keep continuation roots as separate lifecycle records. First
aggregate every physical execution to preserve its root/child classification,
then collapse only root execution groups by canonical conversation ID. Choose a
latest representative root by a total timestamp/ID ordering. Seed fixed,
generic ward instructions through the existing template creator.

## Constraints

- Reuse the current SQLite queries, `StateService`, and ward-layout creator.
- Preserve the existing `root_only` post-aggregation rule.
- Do not add a schema migration, an API endpoint, a UI lifecycle workaround,
  a compatibility path, or role-specific ward configuration.
- Use the exact lowercase-hex continuation suffix grammar; do not canonicalize
  arbitrary `-cont-` text.
- Clamp list limits and bind typed pagination/filter values; no query text may
  originate from a client-supplied string.
- The legacy log list has a maximum page size of 200 and a non-negative typed
  offset.

## Construction tests

- TDD regression fixtures cover multiple continuation roots, canonical status,
  and deterministic representative-root selection.
- TDD creation fixture covers generic `AGENTS.md` only; it does not mutate a
  pre-existing ward.
- Manual QA after the daemon restart: the captured session has one API list row
  and is no longer reported as a running Mission Control session.

## Design (LLD)

### Interfaces & contracts

`LogSession.conversation_id` remains the logical session ID and
`LogSession.session_id` remains a representative execution ID. The status
union adds explicit `unknown` for log-only historical data; otherwise it maps
the richer execution-state lifecycle loss-aware: queued/paused/running →
running and crashed → error. The API and UI type contract change together. Traces to
AC1–AC3.

### State & control flow

Execution state owns lifecycle status. Log aggregation does not make lifecycle
decisions. Mission Control selects only `DelegationType::Root` executions using
`started_at DESC`, then `completed_at DESC`, then `id DESC`, treating a missing
timestamp as older than a present timestamp; it retains the persisted session
status. Log-session root selection uses `started_at DESC`, `ended_at DESC`,
then `session_id DESC`, so a current continuation is not displaced by an older
completed root. This is a representative ordering, not a claim of creation
recency for executions without a persisted creation time. Traces to AC2–AC4.

### Failure, edge cases & resilience

Malformed historical subagent rows continue to be excluded at the physical
execution aggregate before logical-root collapse. Continuation normalization
accepts only a terminal eight lowercase-hex suffix. If no `sessions` row
exists for an old log-only record, its status is explicit `unknown`, never an
invented completion. Traces to AC1–AC3.

### Dependencies & integration

The Rust changes stay in `api-logs`, `execution-state`, and `gateway-services`;
the React transport type accepts the explicit `unknown` log status. Existing
loopback-only gateway exposure and authentication configuration remain the
access boundary; aggregation adds no caller-scoped access path. Traces to
AC1–AC5.

## Declined additions

- Tempted to create a continuation-to-session lookup table; declining because
  canonical conversation IDs already supply the needed grouping key.
- Tempted to add a UI-only stale-state override; declining because lifecycle
  truth belongs to execution state.
- Tempted to make `AGENTS.md` describe default directory roles; declining
  because role semantics remain editable YAML.

## Tasks

### T1: Collapse continuation roots into one logical log-session row

**Depends on:** none

**Touches:** `services/api-logs/src/{repository,service}.rs`

**Tests:**

- TDD: several root execution IDs with one canonical continuation conversation
  return exactly one `root_only` row keyed by the logical `sess-*` ID (AC1–2).
- TDD: subagent rows with mixed/missing parent state remain excluded by the
  physical-execution post-aggregation rule before roots collapse (AC1).
- TDD: every persisted lifecycle status has the documented log-status mapping,
  and a log-only row is `unknown` despite an `ended_at` value (AC3).
- TDD: malformed or non-hex continuation suffixes remain separate (AC6).
- TDD: a limit above 200 is clamped and typed offsets/filters remain parameters
  rather than query text (AC1).

**Approach:** use a physical-execution aggregate CTE, filter children there,
then collapse eligible roots by strictly canonicalized conversation. Select a
stable representative with the documented total ordering and stop recomputing
status in `LogService`.

**Done when:** focused `api-logs` tests pass with the fixture that previously
produced duplicate continuation rows.

### T2: Select the current root for Mission Control

**Depends on:** T1

**Touches:** `services/execution-state/src/repository.rs`

**Tests:**

- TDD: a session with original and continuation roots reports the latest root
  under the documented timestamp/ID ordering while retaining its persisted
  session status, including equal/missing timestamp cases (AC4).

**Approach:** use the existing in-memory session execution list and a
deterministic lifecycle timestamp ordering; do not derive status from it.

**Done when:** focused `execution-state` tests pass and the Mission Control
payload remains wire-compatible.

### T3: Seed template-neutral ward instructions

**Depends on:** none

**Touches:** `gateway/gateway-services/src/ward_layout/create.rs`

**Tests:**

- TDD: default template creation produces non-empty `AGENTS.md` containing
  the template authority and user-editability guidance (AC5).
- TDD: non-AGENTS Markdown nodes retain the generic existing scaffold (AC5).

**Approach:** specialize only a literal root-level conventional `AGENTS.md` at
creation time; its fixed text interpolates no template-owned prose, names no
directory role, and existing wards are untouched. The Linux path creates in a
staging ward and publishes only after every scaffold write succeeds, so a
failed creation is retryable without a partial final ward. It uses a no-replace
`renameat2` publication under the verified wards-root handle. Portable targets
reserve the final directory exclusively before writing and remove it on every
creation error.

**Done when:** focused `gateway-services` ward-layout tests pass.

### T4: Verify API and Mission Control integration

**Depends on:** T1-T3

**Touches:** `apps/ui/src/features/mission-control/**`, `docs/specs/**`

**Tests:**

- Goal-based: existing Mission Control summary mapping tests and TypeScript
  checks accept the explicit `unknown` log status (AC3–4).
- Manual QA: after restarting the daemon, query the captured session and
  confirm a single API row and a terminal Mission Control state (AC1–AC4).

**Approach:** make the minimal transport-type update required by `unknown` and
update completion evidence only after gates pass.

**Done when:** targeted UI checks and manual endpoint verification pass.

## Rollout

Ship directly with a daemon restart. Existing ward files are not migrated or
rewritten. Existing stale `running` sessions are reconciled by the gateway's
existing startup crash recovery; new completions preserve the same state path.
The list query clamps its existing numeric page parameters to a bounded page.

## Risks

- Grouping by conversation changes a historical log projection; regression
fixtures must retain subagent filtering and detail navigation.
- Timestamp ordering must handle missing start/completion values deterministically.
- A failed ward scaffold must never leave an existing final directory that
  blocks the user's retry.

## Changelog

- 2026-07-20: Initial plan from the observed duplicate continuation rows,
  stale Mission Control state, and empty ward instructions.
- 2026-07-20: Review hardened aggregation order, status mapping, continuation
  grammar, pagination, and atomic ward creation requirements.
- 2026-07-20: Shipped after focused Rust/UI verification and clean adversarial
  and security re-reviews.
- 2026-07-20: Hotfix qualified the aggregate sort column and aligned the test
  fixture with the production `sessions.started_at` schema after the live API
  exposed an ambiguity missed by the reduced fixture.
