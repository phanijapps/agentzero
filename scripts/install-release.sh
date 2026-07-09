#!/usr/bin/env bash
#
# zbot release installer for Linux and macOS.
#
# Downloads a prebuilt GitHub Release archive, verifies checksums.sha256, and
# installs zbotd + zbot into a user-local bin directory.

set -euo pipefail

REPO="phanijapps/zbot"
VERSION=""
INSTALL_DIR="${HOME}/.local/bin"
SHARE_DIR="${HOME}/.local/share/zbot"
SERVICE=true
DRY_RUN=false

if [[ -d "${HOME}/Documents" ]]; then
    VAULT_DIR="${HOME}/Documents/zbot"
else
    VAULT_DIR="${HOME}/zbot"
fi

usage() {
    cat <<'USAGE'
Usage: install-release.sh [options]

Options:
  --version <tag>       Install a specific release tag, e.g. v2026.6.1.
  --repo <owner/repo>   GitHub repository to use. Defaults to phanijapps/zbot.
  --install-dir <path>  Directory for zbotd and zbot. Defaults to ~/.local/bin.
  --no-service          Skip Linux systemd user service installation.
  --dry-run             Print detected settings and exit before downloading.
  -h, --help            Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            VERSION="${2:?--version requires a tag}"
            shift 2
            ;;
        --repo)
            REPO="${2:?--repo requires owner/repo}"
            shift 2
            ;;
        --install-dir)
            INSTALL_DIR="${2:?--install-dir requires a path}"
            shift 2
            ;;
        --no-service)
            SERVICE=false
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Required command not found: $1" >&2
        exit 1
    fi
}

detect_platform() {
    local os arch platform_arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux) platform="linux" ;;
        Darwin) platform="macos" ;;
        *)
            echo "Unsupported OS: $os. Use Windows PowerShell install.ps1 on Windows." >&2
            exit 1
            ;;
    esac

    case "$arch" in
        x86_64|amd64) platform_arch="x86_64" ;;
        aarch64|arm64) platform_arch="aarch64" ;;
        *)
            echo "Unsupported architecture: $arch" >&2
            exit 1
            ;;
    esac

    ARCHIVE_PLATFORM="${platform}-${platform_arch}"
}

github_api() {
    local path="$1"
    curl -fsSL \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "https://api.github.com/repos/${REPO}${path}"
}

resolve_version() {
    if [[ -n "$VERSION" ]]; then
        return
    fi
    VERSION="$(github_api /releases/latest | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
    if [[ -z "$VERSION" ]]; then
        echo "Could not resolve latest release for ${REPO}" >&2
        exit 1
    fi
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "Required command not found: sha256sum or shasum" >&2
        exit 1
    fi
}

verify_checksum() {
    local file="$1" checksums="$2" expected actual base
    base="$(basename "$file")"
    expected="$(awk -v name="$base" '$2 == name {print $1}' "$checksums")"
    if [[ -z "$expected" ]]; then
        echo "No checksum entry found for ${base}" >&2
        exit 1
    fi
    actual="$(sha256_file "$file")"
    if [[ "$expected" != "$actual" ]]; then
        echo "Checksum mismatch for ${base}" >&2
        echo "Expected: ${expected}" >&2
        echo "Actual:   ${actual}" >&2
        exit 1
    fi
}

download_release_assets() {
    local archive="$1" tmp="$2" release_url base_url
    release_url="https://github.com/${REPO}/releases/download/${VERSION}"
    base_url="${release_url}"
    curl -fL "${base_url}/${archive}" -o "${tmp}/${archive}"
    curl -fL "${base_url}/checksums.sha256" -o "${tmp}/checksums.sha256"
}

install_binaries() {
    local tmp="$1" archive="$2" root="zbot-${VERSION}"
    mkdir -p "${tmp}/extract" "$INSTALL_DIR" "${SHARE_DIR}/dist" "${VAULT_DIR}/logs"
    tar -xzf "${tmp}/${archive}" -C "${tmp}/extract"
    install -m 755 "${tmp}/extract/${root}/zbotd" "${INSTALL_DIR}/zbotd"
    install -m 755 "${tmp}/extract/${root}/zbot" "${INSTALL_DIR}/zbot"
    rm -rf "${SHARE_DIR}/dist/"*
    cp -R "${tmp}/extract/${root}/dist/." "${SHARE_DIR}/dist/"
}

warn_python_venv() {
    local py tmp output last_error=""

    for py in python3 python; do
        if ! command -v "$py" >/dev/null 2>&1; then
            continue
        fi

        tmp="$(mktemp -d)"
        if output="$("$py" -m venv "${tmp}/venv" 2>&1)"; then
            rm -rf "$tmp"
            return
        fi
        rm -rf "$tmp"
        last_error="${py} -m venv failed: ${output}"
    done

    echo "warning: Python venv support was not found; zbot shell tools may not get ~/Documents/zbot/wards/.venv"
    echo "         On Debian/Ubuntu/Pop!_OS install it with: sudo apt install -y python3 python3-venv"
    if [[ -n "$last_error" ]]; then
        echo "         ${last_error}"
    fi
}

warn_uv() {
    if command -v uvx >/dev/null 2>&1; then
        return
    fi

    echo "warning: uvx was not found; MCP servers that use uvx, such as the bundled Time MCP, will not run until uv is installed"
    echo "         Install uv with: curl -LsSf https://astral.sh/uv/install.sh | sh"
    echo "         Then restart zbot so the service sees ~/.local/bin/uvx."
}

enable_linger() {
    if [[ "$(uname -s)" != "Linux" || "$SERVICE" != "true" ]]; then
        return
    fi
    if ! command -v loginctl >/dev/null 2>&1; then
        echo "loginctl not found; user service may stop after logout"
        return
    fi

    loginctl enable-linger "${USER}" || \
        echo "warning: failed to enable linger; user service may stop after logout"
}

install_linux_service() {
    local unit_dir dist_dir unit_file
    if [[ "$(uname -s)" != "Linux" || "$SERVICE" != "true" ]]; then
        return
    fi
    if ! command -v systemctl >/dev/null 2>&1; then
        echo "systemctl not found; skipping user service installation"
        return
    fi
    if ! systemctl --user status >/dev/null 2>&1; then
        echo "systemctl --user is not available; skipping user service installation"
        return
    fi

    unit_dir="${HOME}/.config/systemd/user"
    dist_dir="${SHARE_DIR}/dist"
    unit_file="${unit_dir}/zbot.service"
    mkdir -p "$unit_dir" "$dist_dir"

    cat > "$unit_file" <<EOF
[Unit]
Description=z-bot daemon (${VERSION})
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=3

[Service]
Type=simple
Environment=PATH=${HOME}/.local/bin:/usr/local/bin:/usr/bin:/bin
ExecStart=${INSTALL_DIR}/zbotd --log-dir ${VAULT_DIR}/logs --log-no-stdout --log-rotation daily --log-max-files 4 --static-dir ${dist_dir}
StandardOutput=journal
StandardError=journal
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

    systemctl --user daemon-reload
    systemctl --user enable --now zbot
}

main() {
    need curl
    need tar
    detect_platform
    resolve_version

    local archive="zbot-${VERSION}-${ARCHIVE_PLATFORM}.tar.gz"

    echo "Repository: ${REPO}"
    echo "Version:    ${VERSION}"
    echo "Platform:   ${ARCHIVE_PLATFORM}"
    echo "Archive:    ${archive}"
    echo "Install:    ${INSTALL_DIR}"

    if [[ "$DRY_RUN" == "true" ]]; then
        exit 0
    fi

    local tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT

    download_release_assets "$archive" "$tmp"
    verify_checksum "${tmp}/${archive}" "${tmp}/checksums.sha256"
    install_binaries "$tmp" "$archive"
    warn_python_venv
    warn_uv
    enable_linger
    install_linux_service

    echo ""
    echo "zbot ${VERSION} installed."
    echo "Binaries: ${INSTALL_DIR}/zbotd and ${INSTALL_DIR}/zbot"
    echo "Dashboard assets: ${SHARE_DIR}/dist"
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        echo "Add ${INSTALL_DIR} to PATH if it is not already available."
    fi
}

main "$@"
