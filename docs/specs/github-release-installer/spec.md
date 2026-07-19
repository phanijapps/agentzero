# Spec: GitHub Release Installer

- **Status:** Closed
- **Owner:** phanijapps
- **Plan:** [`plan.md`](plan.md)
- **Constrained by:** RFC-0004: GitHub Release Installer and Cross-Platform Packaging; `docs/architecture/future-state/path-to-release.md`; `memory-bank/PUBLISHING.md`

> **Spec contract:** this document defines what "done" means. The implementing
> PR must match this spec, or update it. Verification must be derivable from it.

## Objective

Provide a public install path that installs prebuilt zbot release binaries from
GitHub Releases instead of building from source. Linux and macOS users should
be able to run:

```bash
curl -fsSL https://raw.githubusercontent.com/phanijapps/zbot/main/scripts/install-release.sh | bash
```

Windows users should be able to run:

```powershell
irm https://raw.githubusercontent.com/phanijapps/zbot/main/scripts/install.ps1 | iex
```

The installers must detect platform and architecture, resolve latest or pinned
release versions, download the matching artifact, verify its SHA-256 checksum,
install `zbotd` and `zbot`, and leave source-build installation available as a
separate developer path. The release workflow must produce artifacts whose names
and contents match the installer contract.

## Boundaries

The three-tier guard that keeps an implementing agent inside the lines.
*Always do* applies without asking; *Ask first* requires human sign-off
before proceeding; *Never do* is a hard rule, even under time pressure.

### Always do

- Install released binaries from GitHub Releases for the public installer path.
- Keep the source-build installer available under a distinct name such as
  `scripts/install-from-source.sh`.
- Package `zbotd` and `zbot` as the primary binary names.
- Verify `checksums.sha256` before extracting or installing release archives.
- Support pinned installs with `--version <tag>` / `-Version <tag>`.
- Keep `vYYYY.M.D` CalVer tags as an inherited versioning constraint.
- Make installer failure messages explicit when a platform, architecture,
  release, archive, or checksum is unsupported or missing.

### Ask first

- Changing the CalVer tag scheme or artifact version format.
- Hosting the public installer on a non-GitHub domain.
- Installing into privileged system paths by default.
- Adding automatic daemon self-update behavior.
- Making macOS LaunchAgent or Windows service installation default in the first
  implementation slice.
- Dropping source-build installation support.

### Never do

- Never make the public installer build Rust or Node artifacts locally.
- Never install a downloaded archive whose checksum is missing or mismatched.
- Never silently fall back from release install to source build.
- Never package `zerod` or `zero` as the primary release binaries.
- Never require GitHub authentication for the default public install path.
- Never weaken TLS verification in `curl`, PowerShell, or the Rust HTTP stack.

## Testing Strategy

- Release artifact contract: **TDD plus goal-based check**. Archive names and
  contents are deterministic enough for workflow/unit checks, while full
  release publication is verified by a workflow dry run or test tag.
- Linux/macOS installer: **TDD for parsing/selection plus manual QA**. Platform
  detection, version parsing, asset selection, checksum verification, and
  install path logic should be shell-testable; service installation needs a
  Linux manual check.
- Windows installer: **TDD-like PowerShell checks plus manual QA**. Version,
  asset selection, checksum verification, extraction, and PATH behavior should
  be testable on a Windows runner or manual Windows machine.
- Compatibility prerequisites: **goal-based check**. `cargo check` / CI builds
  should prove rustls and Unix-only jemalloc do not block Windows, Linux, and
  macOS release targets.
- Documentation: **goal-based check**. README and publishing docs must show the
  release installer as the public path and the source-build installer as a
  developer path.

## Acceptance Criteria

- [ ] `scripts/install-release.sh` exists and supports Linux x86_64, Linux
  aarch64, macOS x86_64, and macOS aarch64 archive selection.
- [ ] `scripts/install-release.sh` supports latest release resolution and
  `--version <tag>` pinned installs.
- [ ] `scripts/install-release.sh` downloads `checksums.sha256` from the same
  GitHub Release and fails closed on missing or mismatched checksums.
- [ ] `scripts/install-release.sh` installs `zbotd` and `zbot` into a
  user-local install directory by default and supports an install-dir override.
- [ ] Linux service installation is optional and can be disabled with
  `--no-service`.
- [ ] `scripts/install.ps1` exists and installs `zbotd.exe` and `zbot.exe` from
  the Windows release zip after checksum verification.
- [ ] The current source-build installer is preserved under a distinct name and
  is not the public README's first install path.
- [ ] `.github/workflows/release.yml` builds and uploads the five expected
  platform archives plus `checksums.sha256`.
- [ ] Release archives use `zbot-<tag>-<platform>-<arch>` names and contain
  `zbotd`/`zbot` or `zbotd.exe`/`zbot.exe`, `README.md`, `LICENSE`, and
  `VERSION`.
- [ ] Release packaging no longer copies or documents `zerod` / `zero` as the
  primary binary names.
- [ ] Workspace HTTP dependencies use explicit rustls TLS configuration for
  portable packaging.
- [ ] `tikv-jemallocator` and the daemon global allocator are Unix-only so
  Windows MSVC packaging is not blocked by jemalloc.
- [ ] UI assets are either embedded in `zbotd` or included in release archives
  with a documented daemon static-asset fallback.
- [ ] README documents the GitHub raw installer commands for Linux/macOS and
  Windows, and documents source-build install separately.

## Assumptions

- Technical: at spec creation, the release workflow packaged legacy `zerod` and `zero`
  binaries in several steps, while product docs and crate manifests identify
  `zbotd` and `zbot` as the current binaries (source:
  `.github/workflows/release.yml`; `apps/daemon/Cargo.toml`;
  `memory-bank/PUBLISHING.md`).
- Technical: at spec creation, `scripts/install.sh` was a source-build Linux installer
  that requires Rust, Node, GCC, and `make install` (source:
  `scripts/install.sh`).
- Technical: at spec creation, workspace `reqwest` enabled default features rather
  than explicitly selecting `rustls-tls` (source: `Cargo.toml`).
- Technical: at spec creation, daemon depended on `tikv-jemallocator` and installed it as
  global allocator unconditionally (source: `apps/daemon/Cargo.toml`;
  `apps/daemon/src/main.rs`).
- Technical: `docs/architecture/future-state/path-to-release.md` already identifies
  GitHub Releases, `curl | sh`, CalVer, checksums, rustls, jemalloc gating, and
  cross-platform artifacts as release-path work (source:
  `docs/architecture/future-state/path-to-release.md`).
- Product: public install should use GitHub raw installer scripts and GitHub
  Releases, not a product-domain installer URL (source: user confirmation
  2026-06-01).
- Product: versioning should remain out of scope and inherit the existing
  `vYYYY.M.D` CalVer scheme (source: user confirmation 2026-06-01).
- Process: no local `docs/CONVENTIONS.md` or `docs/CHARTER.md` exists; current
  RFC/spec precedent is under `docs/rfc/` and `docs/specs/` (source:
  repository read 2026-06-01).
