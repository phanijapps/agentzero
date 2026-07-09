#!/usr/bin/env bash
#
# z-bot source-build installer for Raspberry Pi / Linux.
#
# Validates prerequisites, builds the daemon and UI, installs as a
# systemd --user service, and enables linger so the daemon survives
# SSH logout and reboots.
#
# Usage:
#   ./scripts/install-from-source.sh
#
# This script is idempotent: re-run after `git pull` to upgrade.
# It never uses sudo. If a prereq is missing, it prints the apt or
# rustup command for you to run yourself.

set -euo pipefail

# ---------------------------------------------------------------------------
# Version + vault path resolution
# ---------------------------------------------------------------------------

# Single source of truth: Cargo.toml [workspace.package].version. The
# first `version = "..."` line in the file (workspace versions appear
# before any per-crate override).
VERSION="$(awk -F\" '/^version[[:space:]]*=/ {print $2; exit}' Cargo.toml)"

# Match the daemon's runtime resolution: `dirs::document_dir()` if it
# exists, else `home_dir()`. Affects log path display only.
if [[ -d "${HOME}/Documents" ]]; then
    VAULT_DIR="${HOME}/Documents/zbot"
else
    VAULT_DIR="${HOME}/zbot"
fi

# ---------------------------------------------------------------------------
# Color output (with NO_COLOR support)
# ---------------------------------------------------------------------------

if [[ -t 1 ]] && [[ -z "${NO_COLOR:-}" ]] && command -v tput >/dev/null 2>&1; then
    GREEN=$(tput setaf 2)
    RED=$(tput setaf 1)
    YELLOW=$(tput setaf 3)
    BOLD=$(tput bold)
    RESET=$(tput sgr0)
else
    GREEN=""; RED=""; YELLOW=""; BOLD=""; RESET=""
fi

ok()    { printf "  %s✓%s %s\n" "$GREEN"  "$RESET" "$1"; }
fail()  { printf "  %s✗%s %s\n" "$RED"    "$RESET" "$1"; }
note()  { printf "%s\n" "$1"; }
header(){ printf "\n%s%s%s\n\n" "$BOLD" "$1" "$RESET"; }

# ---------------------------------------------------------------------------
# Platform check (Linux-only)
# ---------------------------------------------------------------------------

require_linux() {
    if [[ "$(uname -s)" != "Linux" ]]; then
        fail "This installer targets Linux (Raspberry Pi OS, Debian, Ubuntu)."
        note ""
        note "  Detected: $(uname -s)"
        note "  Use scripts/install-release.sh or scripts/install.ps1 for release installs."
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# Prereq checks
#
# Each check_* function:
#   - prints a single status line (✓ or ✗)
#   - returns 0 if the prereq is satisfied, 1 otherwise
#   - emits its fix suggestion to MISSING_FIXES (deferred to end-of-run)
# ---------------------------------------------------------------------------

declare -a MISSING_FIXES

check_rust() {
    if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
        ok "rustc $(rustc --version | awk '{print $2}'), cargo $(cargo --version | awk '{print $2}')"
        return 0
    fi
    fail "rustc + cargo not found"
    MISSING_FIXES+=("Rust toolchain (rustc + cargo):
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    source \$HOME/.cargo/env")
    return 1
}

check_node() {
    if command -v node >/dev/null 2>&1 && command -v npm >/dev/null 2>&1; then
        ok "node $(node --version), npm $(npm --version)"
        return 0
    fi
    fail "node + npm not found"
    MISSING_FIXES+=("Node.js + npm (for UI build):
    sudo apt update && sudo apt install -y nodejs npm")
    return 1
}

check_python_venv() {
    local py tmp output last_error=""

    for py in python3 python; do
        if ! command -v "$py" >/dev/null 2>&1; then
            continue
        fi

        tmp="$(mktemp -d)"
        if output="$("$py" -m venv "${tmp}/venv" 2>&1)"; then
            rm -rf "$tmp"
            ok "$py venv available"
            return 0
        fi
        rm -rf "$tmp"
        last_error="${py} -m venv failed: ${output}"
    done

    fail "python venv support not found"
    MISSING_FIXES+=("Python 3 + venv support:
    sudo apt update && sudo apt install -y python3 python3-venv
    ${last_error:-No python3 or python executable found}")
    return 1
}

check_uv() {
    if command -v uvx >/dev/null 2>&1; then
        ok "uvx $(uvx --version | awk '{print $2}')"
        return 0
    fi

    note "  ! uvx not found (optional; needed for MCP servers that use uvx, such as Time)"
    note "    Install uv with: curl -LsSf https://astral.sh/uv/install.sh | sh"
    note "    Restart zbot after install so the service sees ~/.local/bin/uvx"
    return 0
}

check_gcc() {
    if command -v gcc >/dev/null 2>&1; then
        ok "gcc $(gcc --version | head -1 | awk '{print $NF}')"
        return 0
    fi
    fail "gcc not found (cargo needs a C linker)"
    MISSING_FIXES+=("GCC + build-essential:
    sudo apt install -y build-essential pkg-config")
    return 1
}

check_systemd_user() {
    if command -v systemctl >/dev/null 2>&1 && systemctl --user status >/dev/null 2>&1; then
        ok "systemctl --user is functional"
        return 0
    fi
    fail "systemctl --user is not functional"
    MISSING_FIXES+=("systemd user mode:
    On Pi OS / Debian this should work out of the box. If your distro
    has stripped it down, you may need to enable user lingering or
    install systemd-container. Distro-specific — please consult docs.")
    return 1
}

check_loginctl() {
    if command -v loginctl >/dev/null 2>&1; then
        ok "loginctl available"
        return 0
    fi
    fail "loginctl not found"
    MISSING_FIXES+=("loginctl (part of systemd, used to enable user-service linger):
    Should be present alongside systemctl. If missing, your distro is
    unusual — please check installation.")
    return 1
}

check_disk_space() {
    # cargo build cache for the workspace lands around 1.5 GB.
    # Require 2 GB free in the user's home as a safety margin.
    local available_kb required_kb=2097152
    available_kb=$(df -P "$HOME" | awk 'NR==2 {print $4}')
    if [[ "$available_kb" -ge "$required_kb" ]]; then
        ok "disk free: $((available_kb / 1024 / 1024)) GB"
        return 0
    fi
    fail "disk free: only $((available_kb / 1024 / 1024)) GB (need 2 GB)"
    MISSING_FIXES+=("Disk space:
    Free up at least $((required_kb / 1024 / 1024 - available_kb / 1024 / 1024)) GB
    (cargo's build cache will need ~2 GB).
    Try: cargo clean (if ~/.cargo is large)")
    return 1
}

run_all_checks() {
    local failures=0
    check_rust          || failures=$((failures+1))
    check_node          || failures=$((failures+1))
    check_python_venv   || failures=$((failures+1))
    check_uv
    check_gcc           || failures=$((failures+1))
    check_systemd_user  || failures=$((failures+1))
    check_loginctl      || failures=$((failures+1))
    check_disk_space    || failures=$((failures+1))
    return "$failures"
}

# ---------------------------------------------------------------------------
# Bootstrap (build, install, enable linger, summarize)
# ---------------------------------------------------------------------------

bootstrap() {
    local upgrade_mode=false
    # Detect any prior install (legacy `agentzero.service` or current
    # `zbot.service`). Either case enters upgrade mode.
    if systemctl --user is-enabled agentzero.service >/dev/null 2>&1 \
        || systemctl --user is-enabled zbot.service >/dev/null 2>&1; then
        upgrade_mode=true
    fi

    if "$upgrade_mode"; then
        header "Upgrading z-bot ${VERSION} (this takes ~15 min on a Pi 4)..."
    else
        header "Building z-bot ${VERSION} (this takes ~15 min on a Pi 4)..."
    fi

    note "  → make install (cargo build with ZBOT_INSTALL=1, UI dist, systemd unit)"
    make install >/dev/null

    note "  → loginctl enable-linger ${USER}"
    loginctl enable-linger "${USER}"

    if "$upgrade_mode"; then
        note "  → systemctl --user restart zbot"
        systemctl --user restart zbot
    fi

    note ""
    note "${GREEN}${BOLD}✓ z-bot ${VERSION} is running.${RESET}"
    note ""
    note "  Status:  systemctl --user status zbot"
    note "  Logs:    tail -F ${VAULT_DIR}/logs/*.log"
    note "  URL:     http://zbot.local:18791  (or http://<your-ip>:18791)"
    note ""
    note "  To stop:    make stop"
    note "  To remove:  ./scripts/uninstall.sh"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    header "Checking prerequisites for z-bot ${VERSION}..."
    require_linux
    # shellcheck source=/dev/null
    ok "Linux ($(. /etc/os-release && echo "$PRETTY_NAME"))"

    local failures=0
    run_all_checks || failures=$?

    if [[ "$failures" -gt 0 ]]; then
        header "To install missing prerequisites:"
        for fix in "${MISSING_FIXES[@]}"; do
            note "  $fix"
            note ""
        done
        note "${YELLOW}Re-run ./scripts/install-from-source.sh once these are resolved.${RESET}"
        exit 1
    fi

    bootstrap
}

main "$@"
