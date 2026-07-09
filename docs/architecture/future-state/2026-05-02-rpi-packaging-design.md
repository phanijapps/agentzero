# Raspberry Pi Packaging — Native Install with Systemd User Service

**Status:** Design (awaiting review)
**Date:** 2026-05-02
**Owner:** phanijapps
**Target branch:** `develop`

## Problem

Today, running AgentZero on a Raspberry Pi requires:

1. Cloning the repo, manually installing rustc/cargo/node, running `cargo build --release` and `npm run build`, then figuring out how to launch and persist the daemon.
2. There is no script that validates prerequisites, suggests fixes, or installs a system service.
3. There is no way for an end user to run AgentZero **as a background daemon under their user account** that survives SSH logout, restarts on crash, and produces silent stdout (so SSH sessions aren't polluted by tracing output).

This design adds a Makefile + bootstrap shell scripts that turn the install experience into:

```bash
git clone <repo> && cd zbot
./scripts/install.sh
```

## Goals

1. Single-command install on a fresh Raspberry Pi running Pi OS 64-bit (Bullseye/Bookworm).
2. Daemon runs under the user account (no system services, no `sudo` required to operate).
3. Daemon auto-starts on boot via `systemd --user` + `loginctl enable-linger`.
4. Auto-restarts on crash with a circuit-breaker against pathological loops.
5. Silent stdout/stderr so SSH sessions stay clean; logs land in files only.
6. Re-runnable: `git pull && ./scripts/install.sh` performs a clean upgrade.
7. Symmetric uninstall that leaves user data intact.

## Non-goals

- Cross-compilation from a dev machine (separate follow-up).
- `.deb` packaging (separate follow-up).
- macOS / desktop Linux / Windows installers (different service mechanisms).
- Self-updating daemon (explicit `git pull && install.sh` only).
- System-wide install (everything is user-scoped).
- Size-based log rotation (keeps existing time-based; deferred per scope decision).

## High-level architecture

```text
┌──────────────────────────────────────────────────────────┐
│  User on Raspberry Pi (Pi OS 64-bit)                     │
│                                                          │
│  $ git clone <repo>                                      │
│  $ cd agentzero                                          │
│  $ ./scripts/install.sh                                  │
│       │                                                  │
│       ├─ check_prereqs()  ── rustc, cargo, node, npm,    │
│       │                       gcc, systemd, loginctl,    │
│       │                       2 GB disk free             │
│       │      └─ if missing, prints exact apt commands    │
│       │                                                  │
│       ├─ make build       ── cargo build --release       │
│       │                       cd apps/ui && npm install  │
│       │                       cd apps/ui && npm run build│
│       │                                                  │
│       ├─ make install     ── copies into ~/.local/...    │
│       │                       renders systemd user unit  │
│       │                       daemon-reload + enable+now │
│       │                                                  │
│       └─ loginctl enable-linger ${USER}                  │
│                                                          │
│  Result: agentzero.service running, will auto-restart    │
│  on crash and survive SSH logout / reboot.               │
└──────────────────────────────────────────────────────────┘
```

**Three artifacts:**

- **`Makefile`** — deterministic build/install/uninstall/start/stop/restart/status/logs/clean targets. Plain GNU Make.
- **`scripts/install.sh`** — friendly wrapper. Validates prereqs, suggests fixes for missing ones, then calls `make install` and enables linger.
- **`scripts/uninstall.sh`** — symmetric removal. Stops the unit, removes installed files, leaves `~/Documents/zbot/` intact.

A `Makefile` could in principle do everything, but Make is hostile for "if condition X, suggest Y" prereq UX. The shell script handles UX; Make handles deterministic file operations.

## Install layout

All paths are user-owned XDG. No `sudo` anywhere.

| What | Path | Why |
|---|---|---|
| Binary | `~/.local/bin/zerod` | XDG bin, on `$PATH` by default on Pi OS |
| UI static dist | `~/.local/share/agentzero/dist/` | XDG data |
| systemd unit | `~/.config/systemd/user/agentzero.service` | Standard user-unit location |
| User data (untouched) | `~/Documents/zbot/` | Existing — settings.json, providers, agents, vault |
| Logs | `~/Documents/zbot/logs/` | Daemon's own default — no override needed |
| Source repo | wherever the user cloned it | Build happens here; install copies elsewhere |

## Makefile targets

```make
make build       # cargo build --release && (cd apps/ui && npm install && npm run build)
make install     # build + copy artifacts + render systemd unit + reload + enable+start
make uninstall   # stop/disable unit + remove installed files (keeps user data)
make start       # systemctl --user start agentzero
make stop        # systemctl --user stop agentzero
make restart     # systemctl --user restart agentzero
make status      # systemctl --user status agentzero
make logs        # tail -F ~/Documents/zbot/logs/*.log
make clean       # cargo clean + rm -rf dist
```

`install` is idempotent: fresh systems → fresh install; re-run after `git pull` → upgrade (overwrite binary + dist, regenerate unit, reload, restart).

```make
PREFIX     ?= $(HOME)/.local
BIN_DIR    ?= $(PREFIX)/bin
DIST_DIR   ?= $(PREFIX)/share/agentzero/dist
UNIT_DIR   ?= $(HOME)/.config/systemd/user

.PHONY: build install uninstall start stop restart status logs clean

build:
	cargo build --release
	cd apps/ui && npm install && npm run build

install: build
	install -d $(BIN_DIR) $(DIST_DIR) $(UNIT_DIR)
	install -m 755 target/release/zerod $(BIN_DIR)/zerod
	rm -rf $(DIST_DIR)/* && cp -r dist/* $(DIST_DIR)/
	@sed 's|@@BIN@@|$(BIN_DIR)/zerod|g; s|@@DIST@@|$(DIST_DIR)|g' \
	    scripts/agentzero.service.in > $(UNIT_DIR)/agentzero.service
	systemctl --user daemon-reload
	systemctl --user enable --now agentzero

# … other targets follow the same shape
```

## systemd user unit

Lives at `~/.config/systemd/user/agentzero.service`, rendered from `scripts/agentzero.service.in` at install time.

```ini
[Unit]
Description=AgentZero daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=@@BIN@@ \
    --log-no-stdout \
    --log-rotation daily \
    --log-max-files 4 \
    --static-dir @@DIST@@

# Belt-and-suspenders: silence anything not routed through tracing
StandardOutput=null
StandardError=null

# Auto-restart on crash, with circuit breaker against loops
Restart=on-failure
RestartSec=5
StartLimitIntervalSec=60
StartLimitBurst=3

[Install]
WantedBy=default.target
```

**Choices:**

- **`Type=simple`** — daemon doesn't fork; tokio keeps the process foreground. Correct for a Rust async server.
- **`After=network-online.target`** — daemon binds to `0.0.0.0` and starts mDNS, both need usable interfaces. Soft dependency so a slow network doesn't block boot.
- **`--log-no-stdout`** — existing flag (`apps/daemon/src/main.rs:122`). Stops the daemon writing to stdout. Combined with `StandardOutput=null` / `StandardError=null` to catch panic backtraces and pre-tracing-init output. Two layers of suppression by design.
- **`--log-rotation daily --log-max-files 4`** — uses existing time-based rotation. Keeps 4 daily files. Decision: do not change the appender layer in this scope; user explicitly opted out of size-based rotation.
- **`Restart=on-failure`** — recovers from panics/non-zero exits but not clean shutdowns (e.g., `systemctl stop`).
- **`StartLimitIntervalSec=60` + `StartLimitBurst=3`** — if the daemon crashes 3 times in 60s, systemd stops trying. Prevents a bad config from hammering the Pi.
- **`WantedBy=default.target`** — user services use `default.target` (not `multi-user.target`).

**Deliberately not in the unit:**

- No `Environment=` / `WorkingDirectory=` — daemon resolves data dir via `dirs::document_dir()` correctly without one.
- No hardening (`PrivateTmp`, `ProtectSystem`, etc.) — user services already run unprivileged.
- No resource quotas — too easy to mis-tune across Pi Zero / Pi 4 / Pi 5.
- No `WatchdogSec=` / `sd_notify` — would require code changes inside the daemon.
- No `Nice=` — premature optimization for the Pi 4/5 baseline.

## `scripts/install.sh` (the entry point)

Bash, `set -euo pipefail`. Validates prereqs, suggests fixes, then calls Make.

### Prerequisites checked

| Check | Why | Fix suggestion |
|---|---|---|
| Linux kernel | Script is Linux/Pi-only | (bail with message) |
| `cargo` ≥ 1.75 | Daemon build | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| `node` ≥ 18 + `npm` | UI build | `sudo apt install -y nodejs npm` |
| `gcc` | cargo C linker | `sudo apt install -y build-essential pkg-config` |
| `systemctl --user` | Service install | (usually present; if missing, distro-specific investigation) |
| `loginctl` | Linger | (part of systemd; should always be there) |
| Disk free ≥ 2 GB | cargo build cache | Free space; suggest `cargo clean` if `~/.cargo` is large |

Each check is a function returning 0/1. A wrapper accumulates all failures, reports them all at once, exits non-zero. No partial installs.

The script never `sudo`s. If sudo is needed, it appears in a printed *suggestion* — user runs it themselves.

### UX (missing prereqs)

```
$ ./scripts/install.sh
Checking prerequisites for AgentZero...

  ✓ Linux (Debian 12)
  ✓ rustc 1.83.0
  ✓ cargo 1.83.0
  ✗ node not found
  ✗ npm not found
  ✓ gcc 12.2.0
  ✓ systemctl --user
  ✓ disk free: 14 GB

To install missing prerequisites:

  Node.js and npm (for UI build):
    sudo apt update && sudo apt install -y nodejs npm

Re-run ./scripts/install.sh once these are resolved.
```

### UX (all green)

```
$ ./scripts/install.sh
Checking prerequisites for AgentZero...
  ✓ Linux, rustc, cargo, node, npm, gcc, systemd, disk

Building AgentZero (this takes ~15 min on a Pi 4)...
  → cargo build --release
  → npm install && npm run build
  → installing to ~/.local/{bin,share/agentzero}
  → rendering systemd unit at ~/.config/systemd/user/agentzero.service
  → systemctl --user daemon-reload
  → systemctl --user enable --now agentzero
  → loginctl enable-linger ${USER}

✓ AgentZero is running.

  Status:  systemctl --user status agentzero
  Logs:    tail -F ~/Documents/zbot/logs/*.log
  URL:     http://agentzero.local:18791  (or http://<your-ip>:18791)

To stop:    make stop
To remove:  ./scripts/uninstall.sh
```

### Re-run (upgrade) flow

When the unit already exists, the script:

1. Detects via `systemctl --user is-enabled agentzero` returning success.
2. Shifts the messaging from "Building" to "Upgrading" but the actions are identical.
3. The Make rule's `enable --now` is a no-op when already enabled. Subsequent restart happens via `systemctl --user restart` at the script's end.
4. Linger stays on (no-op call to `loginctl enable-linger` — already enabled).

User flow:

```bash
git pull
./scripts/install.sh
```

User data in `~/Documents/zbot/` is never touched.

## `scripts/uninstall.sh`

Bash, `set -euo pipefail`. Symmetric:

```
1. systemctl --user stop agentzero       (ignore failure if not running)
2. systemctl --user disable agentzero    (ignore failure if not enabled)
3. rm  ~/.config/systemd/user/agentzero.service
4. rm  ~/.local/bin/zerod
5. rm -rf ~/.local/share/agentzero/
6. systemctl --user daemon-reload
```

Deliberately does NOT:
- Remove `~/Documents/zbot/` (user data — config, providers, agents, sessions, vault).
- Disable linger (other user services may rely on it).
- Remove the cloned source repo or `target/` directory.

Final output:

```
✓ AgentZero uninstalled.

To also delete user data: rm -rf ~/Documents/zbot
To disable user-service linger: loginctl disable-linger ${USER}
```

## File structure

```
agentzero/
├── Makefile                              # NEW: targets above
├── scripts/                              # NEW: directory
│   ├── install.sh                        # NEW: prereq check + bootstrap
│   ├── uninstall.sh                      # NEW: symmetric removal
│   └── agentzero.service.in              # NEW: systemd unit template with @@PLACEHOLDERS@@
└── (existing tree unchanged)
```

No code changes outside the new files.

## Testing strategy

Manual, no automated test harness for v1.

### Pre-merge checklist (on a fresh Pi)

- [ ] **Fresh install on clean Pi OS image:** `./scripts/install.sh` → daemon running, web UI loads at `http://agentzero.local:18791` from a phone on the same Wi-Fi.
- [ ] **Reboot test:** `sudo reboot`, wait 60 s, verify daemon auto-started without any user login activity (linger working).
- [ ] **Upgrade test:** `git pull && ./scripts/install.sh` → daemon reflects new code; user data preserved in `~/Documents/zbot/`.
- [ ] **Missing-prereq path:** `apt remove npm`, re-run `install.sh`, verify it lists npm as missing with the apt suggestion and exits cleanly with no partial state.
- [ ] **Uninstall test:** `./scripts/uninstall.sh` → unit gone, binary gone, `~/Documents/zbot/` still present.
- [ ] **Stdout-suppressed verification:** `journalctl --user -u agentzero` returns nothing. `tail -F ~/Documents/zbot/logs/*.log` shows tracing output.
- [ ] **Crash-restart test:** find `zerod` PID and `kill -SEGV` it; watch systemd restart it within 5 s.
- [ ] **Circuit-breaker test:** make the daemon crash 4 times in under 60 s (e.g., temporarily break a config); verify systemd stops restart attempts and `make status` shows the failure state.

## Out of scope for v1 (documented as future work)

- **Cross-compile from a dev laptop.** Add `make rpi-cross` once native install is stable. Toolchain options: `cross`, `cargo-zigbuild`, or `cargo --target aarch64-unknown-linux-gnu` with system gcc-aarch64.
- **`.deb` package.** Layer `cargo-deb` on the cross-compile artifact.
- **macOS / desktop Linux / Windows installers.** Each needs its own service mechanism (launchd, systemd-system, Windows Service).
- **Auto-update from inside the running daemon.** Keep updates as `git pull && install.sh` for now.
- **Watchdog with `WatchdogSec=` and `sd_notify`.** Requires code changes inside the daemon.
- **Resource quotas (`MemoryMax=`, `CPUQuota=`).** Add when a deployment proves it's needed.
- **Size-based log rotation.** Existing time-based rotation suffices; if a deployment proves daily files grow too large, swap appender then.

## References

- `apps/daemon/src/main.rs:60-134` — existing CLI args including `--log-no-stdout`, `--log-rotation`, `--log-max-files`, `--static-dir`.
- `gateway/gateway-services/src/logging.rs` — `LogSettings` struct.
- [systemd.unit(5) — User units](https://www.freedesktop.org/software/systemd/man/systemd.unit.html)
- [`loginctl enable-linger`](https://www.freedesktop.org/software/systemd/man/loginctl.html#enable-linger%20USER%E2%80%A6)
