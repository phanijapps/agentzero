# Plan: GitHub Release Installer

- **Spec:** [`spec.md`](spec.md)
- **Status:** Closed

> **Plan contract:** this is the implementation strategy. Unlike the spec, this
> document is allowed to change as you learn. When it changes substantially
> (a different approach, not just a re-ordering), note why in the changelog
> at the bottom.

## Approach

First make release artifacts trustworthy and predictable, then build installers
against that contract. The release workflow must package the correct binary
names, produce the platform matrix, and publish checksums. The installers should
be thin clients over GitHub Releases: detect platform, choose an artifact,
verify checksum, install binaries, and optionally install a service. Keep
source-build installation separate so the public path never requires Rust or
Node on the user's machine.

## Constraints

- RFC-0004: GitHub raw installer scripts download verified GitHub Release
  binaries.
- Versioning is inherited: use `vYYYY.M.D` tags with no zero padding.
- Public installers must not build from source or silently fall back to source
  build.
- Release packaging must use `zbotd` and `zbot`.
- Checksum mismatch or missing checksum is a hard failure.
- Keep source-build install available as a developer path.

## Construction tests

**Targeted checks:**
- `bash -n scripts/install-release.sh`
- `shellcheck scripts/install-release.sh scripts/install-from-source.sh`
- PowerShell parser/test check for `scripts/install.ps1` on Windows.
- `cargo check -p daemon -p cli --target x86_64-pc-windows-msvc` in CI or on a
  Windows runner.
- Release workflow dry run or test tag that uploads artifacts with the expected
  names.

**Manual verification:**
- Linux: install a pinned release into a temporary directory with
  `--no-service`, then run `zbotd --version` and `zbot --version`.
- macOS: verify archive selection and binary install on Intel or Apple Silicon;
  run the installed binaries.
- Windows: run `install.ps1 -Version <tag> -InstallDir <tmpdir>` and run the
  installed `.exe` files.

## Tasks

### T1: Align release artifact names and contents

**Depends on:** none

**Touches:** `.github/workflows/release.yml`, `memory-bank/PUBLISHING.md`,
`README.md`

**Mode:** Goal-based check

**Spec mapping:** Acceptance Criteria 8, 9, 10.

**Tests:**
- Add or run a workflow validation check that confirms archive names match
  `zbot-<tag>-<platform>-<arch>`.
- Inspect workflow steps to ensure they copy `zbotd` / `zbot`, not `zerod` /
  `zero`.
- Verify each archive contains `README.md`, `LICENSE`, and `VERSION`.

**Approach:**
- Replace release workflow package copy commands for `zerod` / `zero` with
  `zbotd` / `zbot`.
- Rename artifacts from `agentzero-*` to `zbot-*`.
- Add Linux ARM64 target if it is still missing from the workflow.
- Generate `checksums.sha256` over final release archives.
- Keep archive layout consistent across Unix tarballs and Windows zip.

**Done when:** a test tag or workflow dry run produces the five expected
archives plus `checksums.sha256`.

### T2: Remove cross-platform build blockers

**Depends on:** none

**Touches:** `Cargo.toml`, `apps/daemon/Cargo.toml`,
`apps/daemon/src/main.rs`, `Cargo.lock`

**Mode:** Goal-based check

**Spec mapping:** Acceptance Criteria 11, 12.

**Tests:**
- `cargo check -p daemon -p cli`
- `cargo check -p daemon -p cli --target x86_64-pc-windows-msvc` in CI or on a
  Windows runner.
- `cargo tree -i openssl` should not show `reqwest` pulling OpenSSL for normal
  release builds.

**Approach:**
- Change workspace `reqwest` to `default-features = false` with `json`,
  `stream`, and `rustls-tls`.
- Move `tikv-jemallocator` under `[target.'cfg(unix)'.dependencies]`.
- Gate the daemon `#[global_allocator]` with `#[cfg(unix)]`.
- Let Windows use the system allocator.

**Done when:** Windows-target checks no longer fail due to jemalloc or native
  TLS dependencies.

### T3: Decide and implement UI asset packaging

**Depends on:** T1

**Touches:** `apps/daemon/Cargo.toml`, `apps/daemon/src/main.rs`,
`gateway/src/http/mod.rs`, `gateway/src/config.rs`,
`.github/workflows/release.yml`, `apps/ui/vite.config.ts`

**Mode:** Goal-based check plus focused tests

**Spec mapping:** Acceptance Criteria 13.

**Tests:**
- `npm run build` from `apps/ui`.
- `cargo check -p daemon -p gateway`.
- HTTP/router test or smoke check that `/` serves the dashboard when
  `--static-dir` is not supplied in a release-style build.
- HTTP/router test or smoke check that `--static-dir` still overrides embedded
  assets for development.

**Approach:**
- Prefer embedded UI using workspace `rust-embed` and an `embedded-ui` feature.
- Keep `--static-dir` as the first-precedence development override.
- If embedding proves too invasive, package `dist/` inside release archives and
  update installer/service defaults to point the daemon at it.
- Update release workflow so UI build output exists before daemon packaging.

**Done when:** a release-installed `zbotd` can serve the dashboard without a
  source checkout.

### T4: Split source-build installer from release installer

**Depends on:** T1

**Touches:** `scripts/install.sh`, `scripts/install-from-source.sh`,
`scripts/install-release.sh`, `scripts/uninstall.sh`, `README.md`

**Mode:** TDD-like shell checks plus manual QA

**Spec mapping:** Acceptance Criteria 1, 2, 3, 4, 5, 7, 14.

**Tests:**
- `bash -n scripts/install-release.sh scripts/install-from-source.sh`
- `shellcheck scripts/install-release.sh scripts/install-from-source.sh`
- Unit-style shell tests for platform/architecture mapping if a shell test
  harness exists; otherwise factor mapping into small functions and test with
  environment overrides.
- Manual: `scripts/install-release.sh --version <tag> --install-dir <tmpdir>
  --no-service`.

**Approach:**
- Move the current source-build installer to
  `scripts/install-from-source.sh`.
- Create `scripts/install-release.sh` for Linux/macOS release artifact install.
- Support options:
  - `--version <tag>`
  - `--install-dir <path>`
  - `--repo <owner/repo>`
  - `--no-service`
  - `--dry-run`
- Use GitHub Releases API for latest by default and release-by-tag for pinned
  installs.
- Verify checksums before extraction.
- Install Linux systemd user service only when requested/default and available;
  do not make binary install fail just because service setup is unsupported
  when `--no-service` is used.

**Done when:** Linux/macOS installer can install a pinned release without Rust,
  Node, GCC, or a source checkout.

### T5: Add Windows PowerShell installer

**Depends on:** T1, T2

**Touches:** `scripts/install.ps1`, `README.md`, `.github/workflows/release.yml`

**Mode:** TDD-like PowerShell checks plus manual QA

**Spec mapping:** Acceptance Criteria 6, 14.

**Tests:**
- PowerShell parser check on Windows.
- Manual or CI: `scripts/install.ps1 -Version <tag> -InstallDir <tmpdir>`.
- Verify checksum mismatch fails.
- Verify installed `zbotd.exe --version` and `zbot.exe --version` run.

**Approach:**
- Implement Windows release selection for `zbot-<tag>-windows-x86_64.zip`.
- Download and verify `checksums.sha256` using `Get-FileHash`.
- Install binaries to a user-local bin directory by default.
- Add an option to update user PATH, with explicit messaging when skipped.
- Keep Windows service installation out of the first slice unless the spec is
  updated.

**Done when:** Windows users can install released binaries without a source
  checkout.

### T6: Documentation and release-path validation

**Depends on:** T1-T5

**Touches:** `README.md`, `memory-bank/PUBLISHING.md`,
`docs/architecture/future-state/path-to-release.md`, `docs/specs/github-release-installer/*`

**Mode:** Goal-based check

**Spec mapping:** Acceptance Criteria 1-14.

**Tests:**
- README install commands resolve to files that exist in the repo.
- Publishing docs list artifact names that match the workflow.
- `rg -n "zerod|zero" .github/workflows/release.yml README.md
  memory-bank/PUBLISHING.md` only finds intentional legacy notes.
- Run the targeted checks from this plan and record results in the final PR
  summary.

**Approach:**
- Make release installer the first public install path in README.
- Move source-build installation into a developer/install-from-source section.
- Update publishing docs to the new artifact contract.
- Mark relevant future-state packaging lines as implemented or superseded once
  code and CI are in place.

**Done when:** docs, workflow, and installers describe the same public
  packaging contract.

## Rollout

Ship behind a release tag. Keep the old source-build installer available for at
least one release cycle after introducing `install-release.sh`. Announce the new
GitHub raw install command in README and release notes. If a release installer
bug is found, fix the script on `main` for future installs and cut a patch
release for broken artifacts.

## Risks

- GitHub raw installer scripts can change on `main`; pinned script URLs are
  available for users who want reproducibility.
- Checksum verification protects archives but not the bootstrap script itself.
  Signed checksums/provenance should follow as release hardening.
- Linux service setup can vary by distro; keep binary install independent from
  service install.
- macOS and Windows users may expect native service/app integration; this spec
  intentionally starts with binary installation.
- UI embedding may expose release build ordering bugs if UI assets are missing.

## Changelog

- 2026-06-01: initial plan.
- 2026-06-01: implementation chose bundled `dist/` assets in release archives
  rather than embedded UI for the first slice; direct zbot HTTP clients,
  `fastembed`, and daemon allocator configuration were moved to portable
  release defaults.
