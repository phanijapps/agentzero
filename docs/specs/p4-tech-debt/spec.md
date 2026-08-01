# Spec: P4 CI and E2E Debt Cleanup

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** none
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Retire the CI and end-to-end test debt left after P3 without weakening the
required security or test gates: make every UI Playwright spec belong to one
explicitly runnable suite, keep the required deterministic lane fixed at 22
tests, and move GitHub-authored JavaScript actions off deprecated Node 20
runtimes. Retain the narrow React Router RSC advisory exception only until the
vendor publishes the named patched release; do not trade it for a downgrade
with broader known vulnerabilities.

## Boundaries

### Always do

- Keep `npm audit` fail-closed for every unexpected high or critical advisory.
- Keep every UI Playwright spec assigned to exactly one named execution lane.
- Preserve the P3 required lane's daemon lifecycle, fresh-vault isolation, and
  exact 22-test collection.
- Use maintained, official major tags for GitHub-authored actions and preserve
  existing workflow permissions, cache keys, artifacts, and job dependencies.

### Ask first

- Add or remove a test from the required Playwright lane.
- Provision provider credentials or persistent user data in GitHub Actions.
- Change workflow permissions, required-check policy, or coverage tolerance.

### Never do

- Downgrade React Router to an older release with known high-severity
  vulnerabilities.
- Silence, wildcard, or remove the React Router exception's advisory,
  package, expiry, owner, or RSC-source guard.
- Let debug, historical-data, WebSocket-diagnostic, or provider-backed tests
  enter the required deterministic lane implicitly.
- Add a new top-level dependency, module boundary, or CI service.

## Testing Strategy

- **React Router exception:** goal-based security probes compare the installed
  release with the live npm audit report. The removal criterion is deferred
  only while the advisory's patched `8.3.0` release is unavailable.
- **Playwright ownership:** TDD at the configuration boundary. A unit test
  enumerates every `tests/e2e/**/*.spec.ts` file and proves the named projects
  form a complete, duplicate-free partition; Playwright `--list` proves the
  required project still selects exactly 22 tests.
- **Action runtime migration:** goal-based checks reject deprecated action
  majors in the touched workflows, parse the workflow files, and rely on live
  pull-request runs to prove the repository's hosted and self-hosted runners
  support the replacement actions.
- **Integrated behavior:** existing frontend lint, build, unit, integration,
  required E2E, Ward portability, security, and coverage jobs remain the
  release evidence.

## Acceptance Criteria

- [x] Every `apps/ui/tests/e2e/**/*.spec.ts` file belongs to exactly one named
  Playwright project: required, live-daemon, provider-backed, or diagnostic.
- [x] Package scripts expose each Playwright project explicitly, and the
  required CI job invokes the required project rather than repeating a
  separate filename list.
- [x] The required project lists exactly the existing 22 smoke, navigation,
  and persistent-surface tests; debug, historical-data, WebSocket diagnostic,
  and provider-backed specs are absent from that list.
- [x] A unit test fails when an E2E spec is unclassified, classified twice, or
  added to the required project without an intentional expectation update.
- [x] Across `.github/workflows/*.{yml,yaml}`, `actions/checkout`,
  `actions/setup-node`, `actions/setup-python`, `actions/upload-artifact`,
  `actions/download-artifact`, `codecov/codecov-action`,
  `SonarSource/sonarqube-scan-action`, `gitleaks/gitleaks-action`, and
  `softprops/action-gh-release` use the verified Node-24-compatible majors
  `v7`, `v7`, `v7`, `v7`, `v8`, `v7`, `v8`, `v3`, and `v3`, respectively,
  without changing workflow permissions, cache inputs, artifact retention, or
  job ordering.
- [x] The React Router exception remains package- and advisory-specific,
  owner-bound, expiring, and invalidated by RSC source markers; `7.18.2`
  remains installed rather than downgrading to a more broadly vulnerable
  release.
- [ ] Remove the React Router exception (deferred: p4-react-router-830) and
  install patched `8.3.0` or newer once that release is available from npm.
- [x] Frontend lint, build, unit tests, E2E ownership tests, required
  Playwright listing, workflow validation, security scans, and live PR CI pass.

## Assumptions

- Technical: the latest npm-published `react-router-dom` release is `7.18.2`,
  while the reviewed advisory names unpublished `8.3.0` as patched (source:
  npm registry probe and GitHub Advisory Database, 2026-07-30).
- Technical: `7.11.0` is not a safe workaround because an isolated npm audit
  reports multiple high-severity advisories against that dependency graph
  (source: temporary package-lock audit probe, 2026-07-30).
- Technical: the current UI collection is 97 tests in 14 spec files and the
  P3 required lane is 22 tests in three files (source:
  `npx playwright test --list` probes, 2026-07-30).
- Technical: current workflows use Node-20-based action majors and the current
  GitHub-hosted/self-hosted run forced them onto Node 24 successfully (source:
  `.github/workflows/` and PR #232 annotations, 2026-07-30).
- Process: specs use `Draft | Approved | Implementing | Shipped | Archived`
  and plans use `Drafting | Executing | Done` (source:
  `docs/CONVENTIONS.md` spec metadata contract).
- Product: the user approved retaining the narrow exception while completing
  E2E classification and action-runtime upgrades (source: user confirmation
  2026-07-30).

## Verification Evidence

- `npm run test -- tests/integration/playwright-config.test.ts`: the ownership
  guard passed and covered all 14 E2E spec files without duplicate assignment.
- `npm run test:e2e:required -- --list`: 22 tests in the required project;
  the other projects list 47 live-daemon, 21 provider-backed, and 7 diagnostic
  tests.
- `npm run test:e2e:required -- --workers=1`: all 22 required tests passed
  against an isolated daemon.
- `python3 scripts/check_github_action_runtimes.py`: nine governed action
  families passed across six workflows; quoted, case-variant, and malformed
  governed references are inside the fail-closed parser boundary.
- `npm run lint`, `npm run build`, and `npm test`: passed; the unit suite
  reported 1,352 passing tests.
- `node ../../scripts/npm-audit-high.mjs .`: accepted only the scoped,
  unexpired React Router advisory exception.
- `cargo fmt --all --check`, `cargo check --workspace`,
  `cargo clippy --all-targets -- -D warnings`, and `cargo test --workspace`:
  passed.
- Pull request [#233](https://github.com/phanijapps/zbot/pull/233):
  [`Tests` run 30600913546](https://github.com/phanijapps/zbot/actions/runs/30600913546)
  passed unit, integration, coverage, macOS/Windows Ward portability, and the
  required E2E lane; [`Security checks` run
  30600913566](https://github.com/phanijapps/zbot/actions/runs/30600913566)
  passed the action-runtime policy, Rust policy, npm audit, and secret scan.
