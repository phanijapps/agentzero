# P3 CI Stabilization Plan

- **Status:** Executing
- **Spec:** [spec.md](spec.md)

## Decisions

- Execute in the user-approved order: security cache, macOS semantics, Windows stable safety, E2E lifecycle.
- Keep platform-specific Windows FFI behind a safe internal module and a target-specific dependency.
- Keep the E2E daemon in the same workflow step as Playwright so shell cleanup is deterministic.

## Tasks

### T1 — Isolate Cargo state for the security job

Depends on: none  
Tests:

- no stub (goal-based): workflow inspection confirms every scanner remains; `npm ci`, lint, build, unit tests, coverage, and `node scripts/npm-audit-high.mjs apps/ui` validate the refreshed UI toolchain; live CI proves the security job can install and execute Cargo tooling.

Approach: configure the job to use runner-temporary writable Cargo state without removing cache or scanners, refresh vulnerable UI lock selections, upgrade development-tool majors only where the advisory database has no fixed release in the declared range, and keep the React Router RSC advisory as an explicit client-only dashboard exception until npm publishes a clean release line.  
Files: `.github/workflows/security.yaml`, `scripts/npm-audit-high.mjs`, `apps/ui/package.json`, `apps/ui/package-lock.json`, `apps/ui/eslint.config.js`  
Done when: the local Node audit wrapper and live `Rust + Node security scan` job pass with job-scoped Cargo state.

### T2 — Repair portable missing-entry semantics

Depends on: T1  
Tests:

- `portable_entry_counts_distinguish_missing_from_unsafe` asserts the full count-classification matrix for missing, exact, alias, and duplicate entries (AC2); stub: true.
- `required_bounded_vault_component_remains_missing` proves a required absent component returns `BoundedFileError::Missing` rather than being ignored (AC2); stub: true.
- Existing portable Ward creation tests prove the optional legacy probe alone tolerates `Missing`.

Approach: extract/classify exact and case-alias counts, write failing cases first, and return `Missing` only when both counts are zero.  
Files: `gateway/gateway-services/src/ward_layout/loader.rs`  
Done when: the focused unit test and `cargo test -p gateway-services portable_` pass on the supported portable CI hosts.

### T3 — Replace unstable Windows metadata methods

Depends on: T2  
Tests:

- no stub (goal-based): `cargo check -p gateway-services --target x86_64-pc-windows-msvc` proves stable compilation, while focused helper/caller tests and live Windows Ward tests prove the checks remain active.

Approach: add a Windows-only safe wrapper over `GetFileInformationByHandle`, use opened handles for link count and identity, and preserve fail-closed behavior.  
Files: `gateway/gateway-services/Cargo.toml`, `gateway/gateway-services/src/lib.rs`, a focused Windows helper module, Ward loader/usage call sites, `Cargo.lock`  
Done when: the stable Windows cross-target check has no `windows_by_handle` use and the live Windows portable Ward job passes.

### T4 — Give UI E2E a bounded daemon lifecycle

Depends on: T3  
Tests:

- no stub (goal-based): a local bounded daemon health smoke test exercises the command and the explicit smoke/navigation/persistent-surfaces spec set proves the UI is served through Vite without implicitly collecting debug or provider-backed suites.

Approach: start the built daemon with isolated data, poll health with a fixed bound, surface logs on failure, trap cleanup, then run Playwright.  
Files: `.github/workflows/test.yml`  
Done when: the daemon reaches health before Playwright, cleanup runs on exit, and the live E2E job passes without connection-refused retries.

### T5 — Close the loop

Depends on: T4  
Tests:

- no stub (goal-based): workspace gates, spec lint, Engram refresh, three clean implementation reviews, and green PR checks.

Approach: run all mechanical gates, complete adversarial/security/quality review, update spec status, reindex Engram, commit, push, and open a draft PR.  
Files: spec review notes and status, Engram index  
Done when: all gates and reviews are clean and the draft PR is open.

## Risks

- Incorrect Windows FFI field handling could weaken identity checks; mitigate with a tiny wrapper, documented safety invariants, and identity construction from volume plus both file-index halves.
- Cross-platform behavior is partly CI-only; mitigate with pure classification tests and stable cross-target compilation before live CI.
- A self-hosted runner may retain hostile state; mitigate with runner-temporary job state.
- Daemon startup failures can be opaque; mitigate with bounded polling, process liveness checks, and log output.

## Contract Impact

No public API, persistence schema, or user-facing behavior changes. This repairs build, test, and internal filesystem-safety portability.

## Implementation Notes

- 2026-07-30: implemented all tasks and ran local gates: `cargo fmt --all --check`, `cargo check --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, focused `gateway-services` portable tests, `npm ci`, `npm run lint`, `npm run build`, `npm run test`, `npm run test:coverage`, `node scripts/npm-audit-high.mjs apps/ui`, and a local daemon health smoke.
- 2026-07-30: attempted `cargo check -p gateway-services --target x86_64-pc-windows-msvc`; blocked by this Linux host missing Windows C toolchain/linker support for existing native dependencies (`onig_sys`, `ring`, `libsqlite3-sys`), not by `gateway-services` Windows metadata usage.
