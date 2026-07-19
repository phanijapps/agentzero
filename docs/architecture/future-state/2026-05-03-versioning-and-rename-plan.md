# Date-based Versioning + Rename to `zbot` — Plan

**Status:** [IMPLEMENTED 2026-05-03 — PR D (`scripts/release.sh` automation) deferred per design doc `04eed207`]
**Date:** 2026-05-03
**Owner:** phanijapps

PR landing log:
- PR A1 (Cargo `[[bin]]` rename) — `9b5e3e37` [LANDED]
- PR A2 (install scripts + Makefile + auto-embed version) — `6a92f91f` [LANDED]
- PR A3 (UI version badge) — `36ed0cf4` [LANDED]
- PR B (release `2026.5.3`) — `1744c04f` [LANDED]
- PR C (user-copy + mDNS rename) — `7adb5efc` [LANDED]
- PR D (release-on-main workflow) — design doc only at `04eed207` [DEFERRED]

## Three asks bundled

1. **Date-based versioning** — replace the current `0.1.0` semver with a CalVer scheme.
2. **Install scripts** currently say `zerod` / `zero` / `agentzero`; they should say `zbot`.
3. **Plan a low-effort app rename** so the repo stops mixing `agentzero`, `zero`, and `zbot`.

These share a release flow, so handling them together is cheaper than three separate passes.

## Current state — audit

| Surface | Today | Layer |
|---|---|---|
| Workspace version (`Cargo.toml`) | `0.1.0` | Versioning |
| `apps/ui/package.json` version | independent | Versioning |
| Daemon binary | `zerod` (built from `gateway` crate) | Layer 1 |
| CLI binary | `zero` (`apps/cli/Cargo.toml` `name = "zero"`) | Layer 1 |
| Systemd unit | `agentzero.service` (template `scripts/agentzero.service.in`) | Layer 1 |
| mDNS hostname | `agentzero.local` | Layer 1 |
| mDNS instance default | `agentzero` | Layer 1 |
| Make targets | `systemctl --user start agentzero` etc. | Layer 1 |
| Vault dir | `~/Documents/zbot/` (already correct ✓) | n/a |
| GitHub repo | `phanijapps/zbot` (already correct ✓) | n/a |
| Brand string in user copy | mixed: `AgentZero`, `z-bot`, `zbot` | Layer 2 |
| README / docs | mixed | Layer 2 |
| Crate names | `agent-runtime`, `agent-tools`, `gateway` | Layer 3 |
| Type names referencing `AgentZero` | scattered (>100) | Layer 3 |

`rg` totals: 54 files mention `agentzero`, 84 mention `zbot`/`z-bot`, 143 mention bare `zero`.

## Part 1 — Versioning: `YYYY.M.D` (date-only CalVer)

### Choice: `YYYY.M.D`, **no zero-padding**

Examples: `2026.5.3`, `2026.5.4`, `2026.6.1`, `2026.12.31`.

### Why no zero-padding

The version string lives in `Cargo.toml`. Semver explicitly forbids leading zeros in numeric identifiers (`05` is invalid; `5` is valid). Zero-padding the month/day would generate `2026.05.03` which fails strict semver parsers (cargo accepts it leniently today, but other tools in the chain — especially npm-side and CI matchers — may not). Sticking to `YYYY.M.D` keeps every downstream parser happy.

Tradeoff: lexical sort within a year breaks (`2026.5.3` > `2026.10.3` as strings). That's fine — sort by `git tag --sort=creatordate` or numerically by component, not lexically.

### Bump rules

- One release per calendar day. Today's cut → today's date.
- Multiple cuts the same day → bump the day component to "tomorrow's date" or wait. We don't introduce a 4th `PATCH` segment because that breaks the 3-segment semver shape.
- Hotfix on a prior tag: cut a fresh date, no separate "patch" track.

### What carries the version

Single source of truth: `Cargo.toml [workspace.package] version`. Everything else reads from there:

- All workspace crates inherit via `version.workspace = true`
- `apps/ui/package.json` — `npm version 2026.5.3` once per cut
- Daemon `--version` and CLI `--version` — emitted by `env!("CARGO_PKG_VERSION")` (already wired in Rust binaries)
- Systemd unit `Description=` — substituted at install time (see Part 2)
- Git tag — `v2026.5.3`

### Tooling: `scripts/release.sh` (later)

```bash
#!/usr/bin/env bash
# Usage: scripts/release.sh
# Bumps Cargo.toml + package.json to today's date and tags.
set -euo pipefail
TODAY=$(date +%Y.%-m.%-d)   # %-m / %-d strip leading zeros (GNU date)
cargo set-version --workspace "$TODAY"
( cd apps/ui && npm version --no-git-tag-version "$TODAY" )
git add Cargo.toml apps/ui/package.json
git commit -m "release: $TODAY"
git tag -a "v$TODAY" -m "Release $TODAY"
echo "Tagged v$TODAY. Push with: git push --follow-tags"
```

Out of scope for the first version-bump PR. Hand-edit the two version strings the first time; automate next cut.

## Part 2 — Install script rename + auto-embed version

The smallest-blast-radius change. Touches:

- `Makefile` — replace `zerod` binary name with `zbotd`, `agentzero.service` with `zbot.service`, `agentzero` Make targets with `zbot`. ~10 lines.
- `scripts/install.sh` — `zerod` → `zbotd`, `agentzero.service` → `zbot.service`, `agentzero.local` → `zbot.local`, `systemctl --user … agentzero` → `systemctl --user … zbot`. ~15 lines.
- `scripts/uninstall.sh` — same. ~5 lines.
- `scripts/agentzero.service.in` → rename file to `scripts/zbot.service.in`; update `Makefile` to reference the new name. 1 line in Make + git mv.
- `apps/cli/Cargo.toml` — `[[bin]] name = "zero"` → `name = "zbot"`. 1 line.
- `gateway/Cargo.toml` — `[[bin]] name = "zerod"` → `name = "zbotd"`. 1 line.

### Auto-embed the version (single-source from `Cargo.toml`)

Hand-coding the version into the install script is fragile — the rename PR shouldn't pin a version, and every release shouldn't need an install-script edit. Read it once, substitute everywhere.

**1. Read the version from `Cargo.toml` at install time.** In the `Makefile`:

```makefile
VERSION := $(shell awk -F\" '/^version[[:space:]]*=/ {print $$2; exit}' Cargo.toml)
```

(Reads the first `version = "..."` line — that's `[workspace.package].version` since it appears before any per-crate override.) Equivalent in `install.sh`:

```bash
VERSION=$(awk -F\" '/^version[[:space:]]*=/ {print $2; exit}' Cargo.toml)
```

**2. Add a `@@VERSION@@` placeholder** to the systemd template `scripts/zbot.service.in`:

```ini
[Unit]
Description=z-bot daemon (@@VERSION@@)
After=network.target

[Service]
ExecStart=@@BIN@@
Environment=AGENTZERO_STATIC_DIR=@@DIST@@
Restart=on-failure

[Install]
WantedBy=default.target
```

**3. Substitute on install** — extend the existing `sed` chain in the `Makefile`:

```makefile
@sed 's|@@BIN@@|$(BIN_DIR)/zbotd|g; s|@@DIST@@|$(DIST_DIR)|g; s|@@VERSION@@|$(VERSION)|g' \
    scripts/zbot.service.in > $(UNIT_DIR)/zbot.service
```

**4. Display the version** in `install.sh` banners and final-summary output so the user sees what they just installed:

```bash
note "Installing z-bot ${VERSION}..."
# ... install steps ...
note "Installed: z-bot ${VERSION}"
note "  Status:  systemctl --user status zbot"
note "  Logs:    tail -F ~/zbot/logs/*.log"
```

The Rust binaries already self-report (`zbotd --version`, `zbot --version`) via `CARGO_PKG_VERSION` — those need no plumbing.

### Migration for existing installs

Add a one-time migration block in `scripts/install.sh`:

```bash
# Migrate old service name if present
if systemctl --user is-enabled agentzero.service >/dev/null 2>&1; then
    note "Migrating from agentzero.service → zbot.service"
    systemctl --user disable --now agentzero.service || true
    rm -f "${UNIT_DIR}/agentzero.service"
fi
```

Then proceed with the regular install flow which writes `zbot.service`.

## Part 3 — App rename plan (`agentzero`/`zero` → `zbot`)

### Three layers, only ship layers 1 + 2

**Layer 1 — Operationally visible (must rename):**
- Binary names (`zerod`, `zero`)
- Systemd unit and template
- mDNS hostname + instance default name
- Make targets and install/uninstall scripts
- Cargo `[[bin]]` name fields

**Layer 2 — User-visible copy (should rename for consistency):**
- README, CONTRIBUTING, install instructions
- HTTP error messages and version banner
- Wizard / settings page strings that show the brand
- Comments at the top of `Cargo.toml`, `package.json`, key modules
- The handful of "AgentZero" strings in user-facing UI copy → standardize on `z-bot` (or `zbot` — pick one and stick to it; my recommendation: `z-bot` for display, `zbot` for identifiers)

**Layer 3 — Source-internal naming (skip — high effort, no user impact):**
- Crate names: `agent-runtime`, `agent-tools`, `gateway-*` — keep. They describe technical concepts, not the product.
- Internal type/module names containing `AgentZero` — keep, mostly invisible.
- Cargo crate IDs in `Cargo.toml`: `name = "gateway"` etc. Keep.

### Standardization on identifiers

Pick once and stop drifting:
- **Identifier form** (paths, binaries, services, hostnames, env vars): `zbot` — lowercase, no separators.
- **Display form** (user copy, README headings, wizard text): `z-bot` — preserves the playful brand.
- **Never use:** `AgentZero`, `agentzero`, `zerod`, `zero` (as the product name; `zero` as a CLI command is also being renamed).

### Sequencing — start small and iterate

Each PR builds on the last; don't bundle. Reviewer can ship A1 alone and stop if needed.

**PR A1 — Cargo binary rename ONLY** (~5 lines): [LANDED — `9b5e3e37`]
- `apps/cli/Cargo.toml`: `[[bin]] name = "zero"` → `name = "zbot"`
- `gateway/Cargo.toml`: `[[bin]] name = "zerod"` → `name = "zbotd"`
- **Nothing else.** Build artifacts now produce the new names; install scripts still expect old names so this PR alone leaves the system mid-rename. That's intentional — the next PRs make it consistent.
- **Test:** `cargo build --release` produces `target/release/zbotd` and `target/release/zbot`. CI green.

**PR A2 — Install scripts + Makefile + service template** (~30 lines): [LANDED — `6a92f91f`]
- `Makefile` references the new binary names; new `VERSION` macro reads `Cargo.toml`; substitutes `@@VERSION@@` into the systemd template.
- `scripts/install.sh` + `scripts/uninstall.sh`: new binary + service name; migration block; version banner display.
- `git mv scripts/agentzero.service.in scripts/zbot.service.in`; add `@@VERSION@@` placeholder.
- mDNS hostname + instance default name.
- **Test:** fresh install on a Pi produces `zbot.service` with `Description=z-bot daemon (X.Y.Z)` matching `Cargo.toml`'s version. Existing install upgrades cleanly.

**PR B — Versioning bump** (~5 lines): [LANDED — `1744c04f` cut `2026.5.3`]
- `Cargo.toml [workspace.package].version` → today's date (`2026.5.3` or whenever this lands).
- `apps/ui/package.json` version → same.
- Tag `vYYYY.M.D` after merge.

**PR C — Layer 2 (user copy + README)** (~30 files): [LANDED — `7adb5efc` (also did mDNS rename)]
- README, install docs, CONTRIBUTING.
- Wizard / settings page strings.
- HTTP banner / version display.
- Standardize on `z-bot` (display) / `zbot` (identifier).
- **Test:** `rg -i "agentzero"` returns only intentional historical references (this plan doc, CHANGELOG entries about the rename).

**PR D — `scripts/release.sh` automation** (later, optional, ~30 lines): [DEFERRED — see design doc commit `04eed207`]
- The release helper sketched above. Useful once the rhythm of monthly cuts kicks in.

### What does NOT get renamed

- `~/Documents/zbot/` vault dir (already correct)
- `phanijapps/zbot` GitHub repo (already correct)
- Crate names like `agent-runtime`, `agent-tools` (internal abstraction names, not product brand)
- The `agent_id` field on cron jobs / sessions / etc. (a generic concept name, not product brand)
- Stored data formats — no schema change needed; everything keys off `agent_id` strings, not the product name

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| Existing installs break on upgrade | One-time migration in install.sh disables old service before writing new one; idempotent |
| Users have `zerod` in their muscle memory / aliases | Document the rename in CHANGELOG; symlink `zerod → zbotd` for one release if needed (drop after) |
| External tooling pins on the binary name (CI, Docker, etc.) | None known; check before merge |
| `cargo install` users — change in binary name | New version note: `cargo uninstall zero && cargo install --path apps/cli` |
| Search for old name in logs | `grep "zerod\|agentzero"` post-rename helps catch stragglers |

## Suggested execution order

1. **PR #X (this one)** — File this plan doc for review.
2. **PR A1** — Cargo `[[bin]]` rename. ~5 lines. Smallest possible first move.
3. **PR A2** — Install scripts + Makefile + service template + version auto-embed. ~30 lines.
4. **PR B** — Version bump to `YYYY.M.D` (e.g., `2026.5.3`). ~5 lines.
5. **PR C** — Layer 2 user copy. Skim and merge.
6. **PR D (future)** — `scripts/release.sh` automation.
7. **Future** — Layer 3 if it ever feels worth it (probably never).

## Out of scope for now

- Rename of crate IDs (`gateway`, `agent-runtime`, etc.) — high effort, low value.
- A new release-cadence policy (monthly vs. ad-hoc) — pick when first cutting.
- Distribution beyond systemd (Flatpak, Homebrew, etc.) — separate plan.
- Renaming the `zbot` GitHub repo — already correct.
- Brand-mark / logo work — not engineering.
