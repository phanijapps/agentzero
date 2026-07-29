# Plan: Ward Lint Continuation Recovery

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

## Approach

Make ordinary conformance findings part of the existing bounded ward context so
the same agent loop can repair them. Keep fatal executor-construction errors as
errors, but reconcile continuation state before returning. Tighten the generic
planning and builder contracts so planned steps name an available agent and a
builder cannot treat an invalid post-write lint report as completion.

## Constraints

- Reuse the existing loader, linter, context packet, state service, and event
  bus.
- Do not add a repair service, retry worker, schema migration, or typed artifact
  roles.

## Construction tests

- Unit coverage asserts exact report/digest preservation, JSON delimiting,
  adversarial filename isolation, a bounded rendered packet, and byte-equivalent
  valid-ward prompt behavior.
- State integration coverage asserts flag ordering plus exact root error,
  terminal session state, sanitized event text, and idempotent duplicate-event
  handling for fatal continuation setup.
- A temporary-vault integration fixture reproduces a ward made invalid between
  delegated steps and verifies the root can enter a repair continuation.

## Design (LLD)

### Interfaces & contracts

The existing internal `WardLayoutState` remains the contract. A strict,
digest-bound projection of its lint report gains a fixed product-owned repair
instruction followed by a
`<ward-lint-report trusted="false" encoding="json">` block only for an
allowlisted recoverable report. Delimiter characters are unicode-escaped, extra
fields are discarded, and the JSON is never interpreted as a path.

### Failure, edge cases & resilience

Only allowlisted ordinary conformance codes are recoverable model input:
missing required paths, wrong kinds/multiplicity, undeclared Markdown or
directories, and malformed/missing OKF frontmatter/type. Configuration,
integrity, confinement, unsafe-entry, unreadable-input, budget, and internal
failures remain fatal. A database transaction creates at most one queued
continuation root and atomically claims the exact session/agent/execution tuple
only after executor construction succeeds. Fatal setup records a sanitized
stable error on the validated root execution, crashes the session, preserves
the continuation request, and emits the existing error event after persistence.

### Dependencies & integration

The change stays within `gateway-execution`, `execution-state`, the existing
templates, and the ward layout service. No dependency or schema changes.

## Declined additions

- Tempted to add an automatic repair service; declining because the user wants
  a model nudge governed by the editable template.
- Tempted to add retries; declining because deterministic invalid state must be
  repaired, not retried unchanged.
- Tempted to introduce typed source/data/report roles; declining because role
  semantics remain user-owned YAML.

## Tasks

### T1: Recoverable lint reports start a repair-capable executor

**Depends on:** none

**Recommended agent:** builder-agent

**Touches:** `gateway/gateway-execution/src/invoke/executor.rs`, `gateway/gateway-execution/tests/**`

**Tests:**

- TDD: an allowlisted invalid ward builds an executor whose system instruction
  contains the exact bounded finding/digest inside the data delimiter and a
  fixed repair gate (AC 1–2, 7).
- TDD: adversarial path text remains JSON data and the rendered packet stays
  bounded (AC 2).
- TDD: a valid ward injects no repair nudge and preserves its prior prompt
  bytes; unsafe/configuration/budget reports remain fatal (AC 1, 7).
- TDD: valid and repair executors expose identical tool registrations, and
  traversal/symlink findings cannot enter repair mode (AC 7).

**Approach:** Preserve the normalized layout packet, append the lint report as
bounded data, and instruct the active model to lint successfully before normal
completion or delegation.

**Done when:** focused executor tests pass for valid and invalid wards.

### T2: Fatal continuation setup leaves truthful terminal state

**Depends on:** T1

**Recommended agent:** builder-agent

**Touches:** `gateway/gateway-execution/src/runner/{continuation_watcher,session_invoker}.rs`, `gateway/gateway-execution/tests/continuation_watcher_tests.rs`

**Tests:**

- TDD: the continuation flag is consumed only after successful spawn and is
  preserved after fatal setup failure (AC 3).
- TDD: a fatal setup failure records a sanitized error on the exact root
  execution carried by the event, crashes the session, and emits the existing
  error event after persistence (AC 3–4).
- TDD: the exact root is terminal-crashed with the stable error, the session is
  terminal-crashed, and the continuation request remains preserved (AC 3–4).
- TDD: duplicate ready events leave terminal state idempotently terminal
  without consuming the preserved request (AC 4).

**Approach:** Thread the existing `root_execution_id` through the continuation
spawner, atomically create/claim the continuation in the existing state store,
and use the lifecycle state/event path on fatal setup failure without adding
retry machinery.

**Done when:** watcher/state regression tests prove success and failure paths.

### T3: Plans recommend agents and invalid builders cannot claim completion

**Depends on:** T1

**Recommended agent:** builder-agent

**Touches:** `gateway/templates/skills/plan-composer/SKILL.md`, `gateway/templates/agents/{planner-agent,builder-agent}.md`, `gateway/gateway-execution/tests/**`

**Tests:**

- Goal-based: a synthetic live catalog with an arbitrary agent name is included
  in planner context; prompt assertions require every step to recommend a name
  from that catalog and keep capability recommendations separate (AC 5).
- Goal-based: builder instructions require repair/re-lint before completion
  after any invalid lint report (AC 2).

**Approach:** Amend only generic planning/execution instructions; do not name
artifact roles or paths. Reuse the existing dispatch doctrine that checks the
named agent against the current available-agent list and requests replanning
instead of silently substituting a fallback.

**Done when:** template tests and static assertions pass.

### T4: A temporary invalid ward reaches a repair continuation

**Depends on:** T1-T3

**Recommended agent:** builder-agent

**Touches:** `gateway/gateway-execution/tests/**`

**Tests:**

- Integration: a temporary ward gains an undeclared Markdown file between two
  delegated turns and the next root executor starts with the exact repair
  packet instead of returning `Err`.

**Approach:** Reproduce the captured session transition in a confined temporary
vault without mutating user data.

**Done when:** the regression fixture reaches repair mode deterministically.

## Rollout

Ship directly. Existing invalid wards become repairable on their next turn;
fatal setup failures become visibly terminal and can be reactivated by the
normal follow-up path. Repairing the live reproduced ward is a separate manual
verification after the code gates pass.

## Risks

- A model may attempt unrelated work before repair; the system instruction and
  builder completion gate must make repair the only allowed action.
- Fatal setup reconciliation must not emit duplicate terminal events after a
  continuation task has already spawned.

## Changelog

- 2026-07-20: Initial plan from the reproduced `undeclared_markdown`
  continuation deadlock; added per-step recommended-agent requirement.
- 2026-07-20: Review hardening added strict lint projection/digest validation,
  delimiter escaping, atomic identity-bound continuation claims, duplicate-event
  suppression, and persistence-before-error-event semantics.
