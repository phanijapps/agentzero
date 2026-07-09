#!/usr/bin/env bash
# Bring up mock-llm + real zerod + UI for Mode Full. Prints a JSON
# summary to stdout with URLs + run_dir.
#
# Uses a seed data-dir copied from $ZBOT_HOST_DATA_DIR (default
# ~/Documents/zbot) so zerod sees a valid config tree. Provider +
# orchestrator are patched to point at mock-llm; MCPs are cleared so no
# external processes spawn; embeddings fall back to internal bge-small.
set -euo pipefail

FIXTURE="${1:-}"
if [[ -z "$FIXTURE" ]]; then
  echo "usage: boot-full-mode.sh <fixture-name>" >&2
  exit 64
fi

REPO="$(git rev-parse --show-toplevel)"
FIXTURE_DIR="$REPO/e2e/fixtures/$FIXTURE"
if [[ ! -d "$FIXTURE_DIR" ]]; then
  echo "fixture not found: $FIXTURE_DIR" >&2
  exit 65
fi

HOST_DATA_DIR="${ZBOT_HOST_DATA_DIR:-$HOME/Documents/zbot}"
if [[ ! -d "$HOST_DATA_DIR/config" ]]; then
  echo "seed config tree missing: $HOST_DATA_DIR/config" >&2
  echo "set ZBOT_HOST_DATA_DIR to a directory containing a config/ tree" >&2
  exit 66
fi

RUN_DIR="$(mktemp -d -t zbot-e2e-full-XXXXXXXX)"
DATA_DIR="$RUN_DIR/data"
mkdir -p "$DATA_DIR/config" "$DATA_DIR/data"
echo "$RUN_DIR" > /tmp/zbot-e2e-latest-run-dir

# Seed config from host (only the text files — skip wards/shards subdirs
# to keep the copy cheap; zerod recreates anything it needs).
cp "$HOST_DATA_DIR/config/"*.json "$DATA_DIR/config/" 2>/dev/null || true
cp "$HOST_DATA_DIR/config/"*.md "$DATA_DIR/config/" 2>/dev/null || true
# shards + wards are required — copy them too.
for sub in shards wards; do
  if [[ -d "$HOST_DATA_DIR/config/$sub" ]]; then
    cp -r "$HOST_DATA_DIR/config/$sub" "$DATA_DIR/config/"
  fi
done

# One shared venv for the mock server (boot-ui-mode.sh created it).
VENV="$REPO/e2e/.venv"
if [[ ! -x "$VENV/bin/python" ]]; then
  python3 -m venv "$VENV" >&2
  "$VENV/bin/pip" install --quiet --upgrade pip >&2
  "$VENV/bin/pip" install --quiet \
    -r "$REPO/e2e/mock_llm/requirements.txt" >&2
fi
PY="$VENV/bin/python"

pick_port() { "$PY" -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1])"; }
LLM_PORT=$(pick_port)
GATEWAY_HTTP_PORT=$(pick_port)
# zerod serves the WebSocket upgrade on the unified HTTP port at /ws.
# The legacy --ws-port flag is gated behind --legacy-ws-port-enabled and we
# don't bind it here. Surface the unified URL as gateway_ws_url so the UI
# connects to the correct endpoint.
GATEWAY_WS_PORT="$GATEWAY_HTTP_PORT"
UI_PORT=$(pick_port)

# Patch providers.json: add a "provider-mock" entry first in the list so it
# becomes the default. Point every model name the fixture might reference at
# the mock. Empty mcps.json so no external processes spawn.
"$PY" <<PY
import json, sys
from pathlib import Path

cfg = Path("$DATA_DIR/config")
providers_path = cfg / "providers.json"
settings_path = cfg / "settings.json"

mock = {
    "id": "provider-mock",
    "name": "Mock (e2e)",
    "description": "e2e mock-llm",
    "apiKey": "sk-mock",
    "baseUrl": "http://127.0.0.1:$LLM_PORT/v1",
    "models": [
        "gpt-4", "gpt-4o-mini", "glm-4-plus",
        "nemotron-3-super:cloud", "minimax-m2.7:cloud",
        "qwen3.5:cloud", "kimi-k2.5:cloud",
    ],
    "verified": True,
    "isDefault": True,
    "createdAt": "2026-04-20T00:00:00+00:00",
    "rateLimits": {"requestsPerMinute": 600, "concurrentRequests": 10},
    "modelConfigs": {},
}
existing = json.loads(providers_path.read_text()) if providers_path.exists() else []
for p in existing:
    p["isDefault"] = False
providers = [mock] + existing
providers_path.write_text(json.dumps(providers, indent=2))

settings = json.loads(settings_path.read_text()) if settings_path.exists() else {}
execution = settings.setdefault("execution", {})
for sub in ("orchestrator", "distillation", "multimodal"):
    cfg_obj = execution.setdefault(sub, {})
    cfg_obj["providerId"] = "provider-mock"
    cfg_obj.setdefault("model", "gpt-4")
execution["setupComplete"] = True
# Disable embeddings-backed features by pinning internal bge-small.
settings.setdefault("embeddings", {})["backend"] = "internal"
settings["embeddings"]["dimensions"] = 384
settings_path.write_text(json.dumps(settings, indent=2))

# Clear MCPs so no external processes spawn during e2e.
(cfg / "mcps.json").write_text("[]\n")
PY

# Start mock-llm
(
  cd "$REPO"
  exec env PYTHONPATH=. "$PY" -m e2e.mock_llm \
    --fixture "$FIXTURE_DIR" \
    --port "$LLM_PORT" \
    > "$RUN_DIR/mock-llm.log" 2>&1
) &
echo $! > "$RUN_DIR/mock-llm.pid"

for _ in $(seq 1 20); do
  curl -sf "http://127.0.0.1:$LLM_PORT/health" >/dev/null 2>&1 && break
  sleep 0.5
done
if ! curl -sf "http://127.0.0.1:$LLM_PORT/health" >/dev/null; then
  echo "mock-llm failed to start; log:" >&2
  cat "$RUN_DIR/mock-llm.log" >&2
  bash "$(dirname "$0")/teardown.sh" "$RUN_DIR" || true
  exit 73
fi

# Start zerod against the seeded data-dir + replay-backed tools.
DAEMON_BIN="$REPO/target/debug/zerod"
if [[ ! -x "$DAEMON_BIN" ]]; then
  DAEMON_BIN="$REPO/target/debug/zbotd"
fi
if [[ ! -x "$DAEMON_BIN" ]]; then
  echo "daemon binary missing: build apps/daemon first" >&2
  bash "$(dirname "$0")/teardown.sh" "$RUN_DIR" || true
  exit 74
fi
(
  cd "$REPO"
  exec env \
    ZBOT_REPLAY_DIR="$FIXTURE_DIR" \
    ZBOT_REPLAY_STRICT=0 \
    "$DAEMON_BIN" \
      --data-dir "$DATA_DIR" \
      --host 127.0.0.1 \
      --http-port "$GATEWAY_HTTP_PORT" \
      --log-level info \
      --no-dashboard \
      > "$RUN_DIR/zerod.log" 2>&1
) &
echo $! > "$RUN_DIR/zerod.pid"

for _ in $(seq 1 60); do
  curl -sf "http://127.0.0.1:$GATEWAY_HTTP_PORT/api/health" >/dev/null 2>&1 && break
  sleep 1
done
if ! curl -sf "http://127.0.0.1:$GATEWAY_HTTP_PORT/api/health" >/dev/null; then
  echo "zerod failed to start; log tail:" >&2
  tail -60 "$RUN_DIR/zerod.log" >&2
  bash "$(dirname "$0")/teardown.sh" "$RUN_DIR" || true
  exit 72
fi

if [[ ! -f "$REPO/dist/index.html" ]]; then
  (cd "$REPO/apps/ui" && npm run build > "$RUN_DIR/ui-build.log" 2>&1) || {
    echo "UI build failed; log tail:" >&2
    tail -30 "$RUN_DIR/ui-build.log" >&2
    bash "$(dirname "$0")/teardown.sh" "$RUN_DIR" || true
    exit 71
  }
fi
(
  cd "$REPO/apps/ui"
  exec npx vite preview --port "$UI_PORT" --strictPort \
    > "$RUN_DIR/ui.log" 2>&1
) &
echo $! > "$RUN_DIR/ui.pid"

for _ in $(seq 1 40); do
  curl -sf "http://127.0.0.1:$UI_PORT/" >/dev/null 2>&1 && break
  sleep 0.5
done
if ! curl -sf "http://127.0.0.1:$UI_PORT/" >/dev/null; then
  echo "UI preview server failed to start; log tail:" >&2
  tail -30 "$RUN_DIR/ui.log" >&2
  bash "$(dirname "$0")/teardown.sh" "$RUN_DIR" || true
  exit 71
fi

cat <<EOF
{"run_dir":"$RUN_DIR","mock_llm_url":"http://127.0.0.1:$LLM_PORT","gateway_http_url":"http://127.0.0.1:$GATEWAY_HTTP_PORT","gateway_ws_url":"ws://127.0.0.1:$GATEWAY_WS_PORT/ws","ui_url":"http://127.0.0.1:$UI_PORT","data_dir":"$DATA_DIR","fixture":"$FIXTURE"}
EOF
