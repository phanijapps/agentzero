# Plan: P4 CI and E2E Debt Cleanup

- **Spec:** [`spec.md`](spec.md)
- **Status:** Done

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially,
> note why in the changelog at the bottom.

## Approach

Model the UI E2E inventory as four mutually exclusive Playwright projects in
`apps/ui/playwright.config.ts`, expose project-specific npm commands, and make
the required workflow consume the same project definition. Add a Vitest
construction test that walks the E2E tree and rejects missing, duplicate, or
accidentally expanded required assignments. Then update GitHub-authored
JavaScript actions across repository workflows to their maintained
Node-24-compatible majors, preserving every existing input and permission.

Files expected to change: `apps/ui/playwright.config.ts`,
`apps/ui/tests/integration/playwright-config.test.ts`, `apps/ui/package.json`,
`.github/workflows/*.yml`, `.github/workflows/security.yaml`, and the P4
spec/backlog/registry documents. Tests demonstrate the E2E partition, exact
required collection, unchanged frontend behavior, valid workflow syntax, and
live CI. This work does not change application runtime code, test assertions,
provider configuration, workflow permissions, or required-check policy.

Tempted to add a standalone suite registry/runner; declining because
Playwright projects are already the native executable registry. Tempted to
split or rewrite stale tests during classification; declining because
ownership and behavior repair are different changes. Tempted to downgrade
React Router to make the audit superficially clean; declining because the
older graph has broader high-severity exposure.

## Constraints

- Preserve the P3 security and deterministic E2E requirements documented in
  [`../p3-ci-stabilization/spec.md`](../p3-ci-stabilization/spec.md).
- The React Router exception remains governed by
  `scripts/npm-audit-high.mjs` until npm publishes `8.3.0` or newer.
- The existing self-hosted `zbot` runner already executed Node-24-forced
  actions in PR #232.
- No new runtime or development dependency is introduced.

## Construction tests

- `npm run test -- tests/integration/playwright-config.test.ts` proves complete,
  duplicate-free E2E ownership and the exact required file set.
- `npx tsc --noEmit --skipLibCheck --moduleResolution Bundler --module ESNext
  --target ESNext --types vite/client,vitest/globals
  tests/integration/playwright-config.test.ts playwright.config.ts` compiles
  the PLAN-time stub against the existing UI toolchain.
- `npm run test:e2e:required -- --list` reports exactly 22 tests in three files.
- A workflow scan reports no deprecated GitHub-authored action majors within
  the migration scope.
- Live PR `Tests` and `Security checks` workflows pass on the final commit.

## Design (LLD)

### Design decisions

- Playwright projects are the single executable suite registry; package
  scripts and CI select projects by name rather than maintaining filename
  lists in multiple places. Traces to AC1–AC4.
- The Router advisory stays as a fail-closed exception because neither the
  patched release nor a safe downgrade exists. Traces to AC6–AC7.
- GitHub-authored actions move by maintained major tag while their inputs
  remain equivalent. Traces to AC5.

### Component / module decomposition

- `apps/ui/playwright.config.ts`: owns suite names and file membership.
- `apps/ui/tests/integration/playwright-config.test.ts`: guards partition
  completeness, uniqueness, and the required set.
- `apps/ui/package.json`: exposes human/CI entry points per suite.
- `.github/workflows/`: consumes the required project and maintained action
  runtimes.

### State & control flow

`npm run test:e2e:<lane>` selects one Playwright project, which selects only
that lane's files. The required GitHub job starts its isolated daemon, waits
for health, runs `test:e2e:required`, then runs the separate Ward archetype
test exactly as in P3. Other lanes remain explicit operator commands with
their preconditions represented by their names.

### Failure, edge cases & resilience

- An unclassified or duplicate spec fails the unit gate.
- An accidental required-lane expansion fails the exact-set assertion.
- An unsupported action runtime fails live CI rather than being masked.
- A new unexpected audit advisory, expired exception, or RSC marker fails the
  existing audit wrapper.

### Quality attributes (NFRs)

- Determinism: the required lane remains 22 tests and one CI worker.
- Security: no advisory class or workflow permission is broadened.
- Maintainability: adding an E2E spec requires choosing one owner lane.

### Dependencies & integration

- React Router `7.18.2` remains pinned until patched `8.3.0+` is published.
- Playwright and Vitest already exist in the UI dependency graph.
- Current official GitHub action majors require Node 24 action-runtime
  support; PR #232 demonstrated that support.

## Tasks

### T1: Every UI E2E spec has one executable owner lane

**Depends on:** none

**Touches:** apps/ui/playwright.config.ts, apps/ui/tests/integration/playwright-config.test.ts, apps/ui/package.json, .github/workflows/test.yml

**Verification mode:** TDD

**Tests:**
- `stub: true`
- `apps/ui/tests/integration/playwright-config.test.ts` is the compilable red
  stub; it must fail
  until every spec is assigned exactly once and the required set is exact
  (AC1, AC3, AC4).
- `npm run test:e2e:required -- --list` must report 22 tests in three files
  (AC2, AC3).
- Each non-required package command must list only its named project (AC2).

**Approach:**
- Export the four file sets from `playwright.config.ts` and build one Chromium
  project per set with exact `testMatch` entries.
- Add `test:e2e:required`, `test:e2e:live-daemon`,
  `test:e2e:provider-backed`, and `test:e2e:diagnostic` scripts.
- Replace the required workflow's filename list with the required script.

**Done when:** the ownership test passes and the required project lists
exactly 22 tests.

### T2: Required workflows no longer use deprecated GitHub action runtimes

**Depends on:** T1

**Touches:** .github/workflows/*.yml, .github/workflows/security.yaml

**Verification mode:** goal-based check

**Tests:**
- No TDD stub (goal-based configuration change).
- Run `python3 scripts/check_github_action_runtimes.py`; it scans
  `.github/workflows/*.{yml,yaml}` for superseded majors of
  `actions/checkout`, `actions/setup-node`, `actions/setup-python`,
  `actions/upload-artifact`, `actions/download-artifact`,
  `codecov/codecov-action`, `SonarSource/sonarqube-scan-action`,
  `gitleaks/gitleaks-action`, and `softprops/action-gh-release`.
- Parse every touched workflow and run available workflow lint.
- Run the complete local frontend and repository mechanical gates before
  relying on live CI.

**Approach:**
- Add `scripts/check_github_action_runtimes.py` as the durable fail-closed
  runtime-major gate and invoke it from the security workflow.
- Update the action families named above to `v7`, `v7`, `v7`, `v7`, `v8`,
  `v7`, `v8`, `v3`, and `v3`, respectively; each target tag's `action.yml`
  declares `node24` or is a composite action without a deprecated Node
  runtime.
- Preserve names, permissions, inputs, cache paths, artifact names,
  retention, conditions, and job dependencies.
- Leave third-party actions unchanged unless their own maintained release is
  required to eliminate the same runtime warning and can be verified.

**Done when:** the workflow scan and syntax validation pass and live hosted
and self-hosted jobs start successfully.

### T3: P4 evidence and deferred Router removal are durable

**Depends on:** T1, T2

**Touches:** docs/specs/p4-tech-debt/**, docs/specs/README.md, docs/backlog.md

**Verification mode:** goal-based check

**Tests:**
- No TDD stub (documentation/evidence task).
- The spec status linter resolves `p4-react-router-830`.
- Final local gates and live PR checks are recorded in the spec.

**Approach:**
- Keep AC7 deferred until npm publishes the vendor's patched release.
- Record verification commands and live workflow evidence.
- Move spec/plan statuses only when their corresponding gates are true.

**Done when:** the spec metadata linter passes and all non-deferred acceptance
criteria carry evidence.

## Rollout

One pull request. E2E project selection and action-major changes take effect
only in CI or explicit local test commands. Rollback is a normal revert; no
data, API, or deployment migration exists.

## Risks

- A Playwright file can mix mocked and provider-backed cases; classification
  is by the strongest prerequisite of the file until a later behavior-focused
  split.
- Node-24-compatible action majors can require a newer self-hosted runner;
  live security CI is the authoritative compatibility gate.
- Broad action replacement can accidentally alter cache defaults; explicit
  existing cache inputs are retained and reviewed.
- The Router exception can outlive its justification if the deferred item is
  not removed promptly after `8.3.0` publishes; its expiry and backlog anchor
  both fail visibly.

## Changelog

- 2026-07-30: Initial plan; Router removal deferred because the advisory's
  patched `8.3.0` release is not available from npm and `7.11.0` has broader
  known high-severity vulnerabilities.
