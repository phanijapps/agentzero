#!/usr/bin/env bash
# ===========================================================================
# z-Bot Docker Launcher
# One-command start for z-Bot in Docker
# ===========================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo ""
echo -e "${GREEN}  ╔══════════════════════════════╗${NC}"
echo -e "${GREEN}  ║         z-Bot Docker          ║${NC}"
echo -e "${GREEN}  ║   Goal-Oriented AI Agent      ║${NC}"
echo -e "${GREEN}  ╚══════════════════════════════╝${NC}"
echo ""

# Check Docker
if ! command -v docker &>/dev/null; then
    echo -e "${RED}Error: Docker is not installed.${NC}"
    echo "Install Docker: https://docs.docker.com/get-docker/"
    exit 1
fi

if ! docker compose version &>/dev/null; then
    echo -e "${RED}Error: docker compose is not available.${NC}"
    echo "Install Docker Compose: https://docs.docker.com/compose/install/"
    exit 1
fi

# Setup .env on first run
if [[ ! -f .env ]]; then
    echo -e "${YELLOW}First run detected — setting up configuration...${NC}"
    echo ""

    # Prompt for vault path
    DEFAULT_VAULT="$HOME/zbot"
    read -rp "Vault path [$DEFAULT_VAULT]: " VAULT_INPUT
    VAULT_PATH="${VAULT_INPUT:-$DEFAULT_VAULT}"

    # Expand ~ to actual home
    VAULT_PATH="${VAULT_PATH/#\~/$HOME}"
    mkdir -p "$VAULT_PATH"

    # Write .env
    cat > .env <<EOF
VAULT_PATH=$VAULT_PATH
HTTP_PORT=18791
NGROK_AUTHTOKEN=
NGROK_DOMAIN=
NGROK_WEB_PORT=4040
EOF

    echo ""
    echo -e "Vault: ${GREEN}$VAULT_PATH${NC}"
    echo -e "HTTP:  ${GREEN}http://localhost:18791${NC}"
    echo -e "WS:    ${GREEN}ws://localhost:18791/ws${NC}"
    echo ""
fi

# Source .env for display
source .env

# Expand ~ in VAULT_PATH and ensure Docker does not create it as root.
DISPLAY_VAULT="${VAULT_PATH/#\~/$HOME}"
mkdir -p "$DISPLAY_VAULT"

# Build and start
echo -e "${YELLOW}Starting z-Bot...${NC}"

if ! docker compose images zbot 2>/dev/null | grep -q zbot; then
    echo -e "${YELLOW}First build — this takes ~10 minutes (Rust compilation)...${NC}"
    echo -e "${YELLOW}Subsequent starts will be instant.${NC}"
    echo ""
fi

docker compose up -d --build

echo ""
echo -e "${GREEN}z-Bot is running!${NC}"
echo ""
echo -e "  Web UI:  ${GREEN}http://localhost:18791${NC}"
echo -e "  Vault:   ${DISPLAY_VAULT}"
echo ""
echo -e "  Stop:    ${YELLOW}docker compose down${NC}"
echo -e "  Logs:    ${YELLOW}docker compose logs -f${NC}"
echo -e "  Rebuild: ${YELLOW}docker compose up -d --build${NC}"
echo ""
echo -e "${YELLOW}⚡ Token Warning:${NC} z-Bot is token-intensive. A single task can"
echo "  consume 100+ LLM calls. Monitor your provider usage."
echo ""
