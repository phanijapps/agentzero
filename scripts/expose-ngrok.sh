#!/usr/bin/env bash
# Expose a host-running z-Bot daemon through ngrok.
#
# This is the dedicated-machine equivalent of docker/docker-compose.yml's
# optional ngrok profile. It tunnels the daemon's unified HTTP + WebSocket
# endpoint, normally http://127.0.0.1:18791.

set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

ZBOT_HOST="${ZBOT_HOST:-127.0.0.1}"
ZBOT_PORT="${ZBOT_PORT:-18791}"
ZBOT_SCHEME="${ZBOT_SCHEME:-http}"
ZBOT_HEALTH_PATH="${ZBOT_HEALTH_PATH:-/api/health}"
NGROK_DOMAIN="${NGROK_DOMAIN:-}"
NGROK_AUTHTOKEN="${NGROK_AUTHTOKEN:-}"
SKIP_HEALTH_CHECK=0
YES=0

usage() {
  cat <<'EOF'
Usage: scripts/expose-ngrok.sh [options]

Expose a z-Bot daemon running directly on this machine through ngrok.

Options:
  --host <host>              Local daemon host. Default: 127.0.0.1
  --port <port>              Local daemon port. Default: 18791
  --url <url>                Full local daemon URL, e.g. http://127.0.0.1:18791
  --domain <domain>          Reserved ngrok domain, e.g. example.ngrok.app
  --authtoken <token>        ngrok authtoken. Prefer NGROK_AUTHTOKEN env.
  --skip-health-check        Start ngrok without checking /api/health first.
  -y, --yes                  Skip the public exposure confirmation prompt.
  -h, --help                 Show this help.

Environment:
  ZBOT_HOST                  Same as --host.
  ZBOT_PORT                  Same as --port.
  ZBOT_SCHEME                Local daemon scheme. Default: http.
  ZBOT_HEALTH_PATH           Health check path. Default: /api/health.
  NGROK_AUTHTOKEN            Optional ngrok authtoken.
  NGROK_DOMAIN               Optional reserved ngrok domain.

Examples:
  scripts/expose-ngrok.sh
  NGROK_AUTHTOKEN=... scripts/expose-ngrok.sh --domain my-zbot.ngrok.app
  scripts/expose-ngrok.sh --url http://127.0.0.1:18791 --yes
EOF
}

fail() {
  echo -e "${RED}Error:${NC} $*" >&2
  exit 1
}

need_arg() {
  local flag="$1"
  local value="${2:-}"
  if [[ -z "$value" || "$value" == --* ]]; then
    fail "$flag requires a value"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)
      need_arg "$1" "${2:-}"
      ZBOT_HOST="$2"
      shift 2
      ;;
    --port)
      need_arg "$1" "${2:-}"
      ZBOT_PORT="$2"
      shift 2
      ;;
    --url)
      need_arg "$1" "${2:-}"
      if [[ "$2" =~ ^([^:]+)://([^:/]+):([0-9]+)$ ]]; then
        ZBOT_SCHEME="${BASH_REMATCH[1]}"
        ZBOT_HOST="${BASH_REMATCH[2]}"
        ZBOT_PORT="${BASH_REMATCH[3]}"
      else
        fail "--url must look like http://127.0.0.1:18791"
      fi
      shift 2
      ;;
    --domain)
      need_arg "$1" "${2:-}"
      NGROK_DOMAIN="$2"
      shift 2
      ;;
    --authtoken)
      need_arg "$1" "${2:-}"
      NGROK_AUTHTOKEN="$2"
      shift 2
      ;;
    --skip-health-check)
      SKIP_HEALTH_CHECK=1
      shift
      ;;
    -y|--yes)
      YES=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "Unknown option: $1"
      ;;
  esac
done

[[ "$ZBOT_PORT" =~ ^[0-9]+$ ]] || fail "port must be numeric"

if ! command -v ngrok >/dev/null 2>&1; then
  fail "ngrok is not installed or not on PATH. Install it from https://ngrok.com/download"
fi

LOCAL_URL="${ZBOT_SCHEME}://${ZBOT_HOST}:${ZBOT_PORT}"
HEALTH_URL="${LOCAL_URL}${ZBOT_HEALTH_PATH}"
WS_SCHEME="ws"
if [[ "$ZBOT_SCHEME" == "https" ]]; then
  WS_SCHEME="wss"
fi

if [[ "$SKIP_HEALTH_CHECK" -eq 0 ]]; then
  if ! command -v curl >/dev/null 2>&1; then
    fail "curl is required for the daemon health check"
  fi
  if ! curl -fsS --max-time 3 "$HEALTH_URL" >/dev/null; then
    cat >&2 <<EOF
${RED}z-Bot daemon is not reachable at ${HEALTH_URL}.${NC}

Start it first, for example:
  systemctl --user start zbot
  # or, from the repo after building the UI:
  cargo run -p daemon --release -- --static-dir ./dist

Then rerun this script.
EOF
    exit 1
  fi
fi

echo ""
echo -e "${GREEN}z-Bot ngrok exposure${NC}"
echo -e "  Local daemon: ${GREEN}${LOCAL_URL}${NC}"
echo -e "  Local WS:     ${GREEN}${WS_SCHEME}://${ZBOT_HOST}:${ZBOT_PORT}/ws${NC}"
echo -e "  Inspector:    ${GREEN}http://127.0.0.1:4040${NC}"
if [[ -n "$NGROK_DOMAIN" ]]; then
  echo -e "  Domain:       ${GREEN}${NGROK_DOMAIN}${NC}"
else
  echo -e "  Domain:       ${YELLOW}temporary ngrok URL${NC}"
fi
echo ""
echo -e "${YELLOW}Warning:${NC} this exposes your local z-Bot daemon on the public internet."
echo "Anyone with the ngrok URL can reach the web UI and API unless you add"
echo "access controls outside z-Bot."
echo ""

if [[ "$YES" -eq 0 && -t 0 ]]; then
  read -rp "Continue? [y/N]: " answer
  case "$answer" in
    y|Y|yes|YES) ;;
    *) echo "Aborted."; exit 0 ;;
  esac
fi

cmd=(ngrok http)
if [[ -n "$NGROK_DOMAIN" ]]; then
  cmd+=(--url "$NGROK_DOMAIN")
fi
cmd+=("$LOCAL_URL")

echo -e "${YELLOW}Starting:${NC} ${cmd[*]}"
echo ""

if [[ -n "$NGROK_AUTHTOKEN" ]]; then
  NGROK_AUTHTOKEN="$NGROK_AUTHTOKEN" exec "${cmd[@]}"
fi

exec "${cmd[@]}"
