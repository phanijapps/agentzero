# Spec: P3 CI Stabilization

- **Status:** Shipped
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** [RFC-0017](../../rfc/0017-ward-layout-archetypes.md), [RFC-0018](../../rfc/0018-filesystem-authoritative-llm-wiki-wards.md), [ADR-0002](../../adr/0002-select-complete-ward-archetypes-at-creation.md), [ADR-0003](../../adr/0003-use-filesystem-authoritative-llm-wiki-wards.md)
- **Brief:** none
- **Contract:** none
- **Shape:** mixed

## Objective

Restore the four failing required CI paths on `develop` without weakening supply-chain checks, filesystem confinement, hard-link protection, file-identity verification, or end-to-end coverage.

## Context

CI on the merged P2 baseline (`5808c46f`) has four independent failures:

1. The self-hosted security runner cannot write Cargo's default git cache under `/home/runner/.cargo`.
2. macOS Ward creation treats an absent optional legacy-template path component as unsafe instead of missing.
3. Windows compilation uses unstable `std::os::windows::fs::MetadataExt` by-handle methods.
4. The UI E2E job starts Vite but not `zbotd`, so `/api/health` is proxied to a closed port until the job times out.

After restoring writable Cargo state, a local run of the unchanged Node audit also exposed newly published high/critical advisories in versions already selected by the UI lockfile. Fixed releases require refreshing the lockfile and upgrading affected development-tool majors; the audit must not be weakened.

## Assumptions

- The four failures reproduce on the merged `develop` baseline and are outside the completed P2 SQLite scope. Verified from GitHub Actions logs and the local history on 2026-07-30.
- The Ward portability failures entered with the same Ward filesystem-safety implementation. Verified from commit history and call-graph inspection on 2026-07-30.
- The UI E2E suite requires the daemon on `127.0.0.1:18791`, while Playwright starts only the Vite server. Verified from the workflow and Playwright configuration on 2026-07-30.
- The user confirmed the recommended four-part sequence on 2026-07-30.

## Boundaries

### Always

- Preserve all Rust, Node, secret, license, and advisory security scans.
- Preserve canonical-path confinement, single-link enforcement, and opened-file identity checks.
- Use stable Rust APIs and a narrowly scoped Windows system wrapper where the standard library remains unstable.
- Give E2E startup a bounded readiness check, actionable daemon logs, and reliable cleanup.

### Ask first

- Change which checks are required.
- Broaden the work beyond the four observed CI blockers.
- Change Ward filesystem policy or supported platforms.

### Never

- Disable, ignore, or mark failing checks as allowed to fail.
- Adopt nightly Rust to access unstable Windows metadata APIs.
- Remove symlink, hard-link, or file-identity defenses.
- Treat a larger timeout or extra test retries as the E2E fix.
- Reuse a persistent, runner-global Cargo home for an isolated CI job.

## Requirements

### R1 — Writable security tooling state

The security job MUST use a writable, job-scoped Cargo home while retaining Rust cache behavior and every existing scan.

The UI lockfile MUST select non-vulnerable releases so the retained high-severity Node audit passes. Where no fixed release exists within the declared development-tool range, a major upgrade MUST pass install, lint, build, unit-test, and coverage gates. If npm has no clean release line for a dependency, the security job MAY carry a narrowly documented advisory exception that still fails on every unexpected high-or-critical advisory.

### R2 — Correct portable path classification

Portable component lookup MUST classify zero exact matches and zero case aliases as missing. It MUST continue to reject case aliases, duplicate exact entries, and ambiguous entries as unsafe.

`Missing` MUST remain fatal for every required Ward component. Only an explicitly optional legacy-template probe may convert it to absence, and only after the containing vault root has passed confinement checks.

### R3 — Stable Windows file safety

Windows builds MUST compile on stable Rust. Link-count and file-identity validation MUST use information from opened handles and MUST fail closed when the operating-system query fails.

The Windows implementation MAY add a target-specific `windows-sys` dependency constrained to the compatible version already present in the lockfile. Any unsafe call MUST be isolated, documented, and expose a safe internal interface.

### R4 — Deterministic E2E service lifecycle

The bounded deterministic E2E job MUST start the built daemon before Playwright, use an isolated temporary data directory, wait for `/api/health` within a bounded interval, run only the explicit smoke, navigation, and persistent-surface specs, print daemon logs on startup or test failure, and stop the daemon when the step exits. Debug, dashboard-era, and provider-backed suites MUST NOT be selected implicitly by this bounded lane.

## Acceptance Criteria

- [x] The security workflow uses a writable per-job Cargo home, its UI lockfile has no unaccepted high-or-critical Node advisories, and it still runs fmt, Clippy, Rust audit/deny, Node audit, and secret scanning.
- [x] Portable-path tests prove that an absent component is `Missing`, while aliases and duplicates remain `Unsafe`; required callers still fail on `Missing`, and only the confined optional legacy-template probe tolerates it.
- [x] `gateway-services` cross-checks for Windows on stable Rust without `windows_by_handle`, and opened-file single-link and identity checks remain enforced.
- [x] The UI E2E step starts `zbotd`, reaches health readiness or fails quickly with logs, and runs the explicit bounded smoke/navigation/persistent-surface specs instead of implicitly collecting debug, dashboard-era, or provider-backed suites.
- [x] No required audit, security control, supported platform, or bounded deterministic E2E assertion is skipped; no timeout-only or nightly-toolchain workaround is introduced.
- [x] Formatting, workspace check, Clippy, tests, relevant cross-target checks, and live PR CI pass.

## Verification Evidence

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test -p gateway-services portable_`
- `cargo test -p gateway-services required_bounded_vault_component_remains_missing`
- `npm ci`
- `npm run lint`
- `npm run build`
- `npm run test`
- `npm run test:coverage`
- `node scripts/npm-audit-high.mjs apps/ui`
- `cd apps/ui && npm run test:e2e -- tests/e2e/smoke.spec.ts`
- `cd apps/ui && npm run test:e2e -- tests/e2e/navigation.spec.ts`
- `cd apps/ui && npm run test:e2e -- tests/e2e/persistent-surfaces.spec.ts`
- local daemon health smoke matching the E2E workflow shape: build `zbotd`, start with isolated data and `--static-dir dist`, poll `/api/health`, then clean up.
- GitHub Actions Security checks run
  [`30597312890`](https://github.com/phanijapps/zbot/actions/runs/30597312890)
  passed on commit `a8228ca8`, including fmt, Clippy, Cargo audit/deny,
  the Rig boundary check, the narrowed Node audit policy, and Gitleaks.
- GitHub Actions Tests run
  [`30597312840`](https://github.com/phanijapps/zbot/actions/runs/30597312840)
  passed on commit `a8228ca8`, including Linux unit/integration/coverage,
  macOS and Windows Ward portability, the 22-test bounded UI E2E lane,
  and the fresh-vault Ward archetype E2E.

The local Linux environment cannot complete `cargo check -p gateway-services --target x86_64-pc-windows-msvc` because existing native C dependencies require MSVC linker/toolchain programs (`lib.exe`) and Windows-target C build support. The code no longer uses `windows_by_handle`; the passing Windows Actions job is the platform proof for stable Windows compilation and handle-based safety tests.

## Failure and Recovery

- A Windows handle query error is returned to the caller and handled as an unsafe/unavailable file, never as success.
- A daemon that exits before readiness or fails to become healthy within the bounded wait terminates the E2E step after printing its log.
- CI state is isolated under runner-provided temporary storage so failed jobs do not poison a shared tool cache.

## Security Considerations

This work crosses path/file, supply-chain, and CI-configuration boundaries. The intended changes retain fail-closed file validation, restrict unsafe Windows FFI to one wrapper around `GetFileInformationByHandle`, avoid path-only identity decisions, and keep all dependency and secret checks intact.

The temporary exception for
[`GHSA-qwww-vcr4-c8h2`](https://github.com/advisories/GHSA-qwww-vcr4-c8h2)
is owned by the z-Bot maintainers and expires on 2026-10-31. The audit wrapper
fails after that date, fails if unstable React Router RSC markers appear in the
UI, and continues to fail for every other high-or-critical advisory. P4 must
remove the exception by adopting the fixed React Router line and its required
Node and React versions.

The broader UI Playwright collection is not a bounded PR gate today: several
files are debug probes or require persistent sessions, a configured provider,
or WebSocket infrastructure. P4 must classify those specs into explicitly
provisioned jobs and modernize stale dashboard-era assertions. P3 keeps the
required PR lane deterministic by naming the smoke, navigation, and
persistent-surface contracts directly, rather than relying on `--grep-invert`,
which filters test titles and never excluded the `long-running/` directory by
path.

## Testing Strategy

- Test-driven unit cases for portable entry classification.
- Windows stable cross-target compilation plus focused file-safety tests where host capabilities permit.
- Focused Ward creation tests on supported CI hosts.
- A bounded local daemon health smoke test and the explicit smoke, navigation, and persistent-surface Playwright specs.
- Full workspace formatting, check, Clippy, and tests, followed by live GitHub Actions verification.

## Declined Patterns

- Replacing identity checks with path strings or canonicalization alone.
- Dropping the Windows link-count check.
- Disabling Rust cache or security scans without evidence that they are the cause.
- Adding a general-purpose process supervisor for one CI lifecycle.
- Increasing the 30-minute E2E timeout.
