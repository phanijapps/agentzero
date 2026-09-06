"""Credential-free MCP fixture for native SDK lifecycle contract probes."""

import json
import sys
import os
import time

mode = os.environ.get("PROBE_MODE", "normal")
if os.environ.get("PROBE_PID_FILE"):
    with open(os.environ["PROBE_PID_FILE"], "a") as output:
        output.write(str(os.getpid()) + "\n")
if mode == "startup_hang":
    time.sleep(60)
if mode == "startup_fail":
    sys.exit(2)

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if "id" not in request:
        continue
    if method == "initialize":
        result = {
            "protocolVersion": request["params"]["protocolVersion"],
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "lifecycle-probe", "version": "1"},
        }
    elif method == "tools/list":
        result = {"tools": [{
            "name": "echo",
            "description": "echo a fixture value",
            "inputSchema": {"type": "object", "properties": {"value": {"type": "string"}}},
        }]}
    elif method == "tools/call":
        if mode == "eof":
            sys.exit(0)
        if mode == "call_hang":
            with open(os.environ["PROBE_CALL_FILE"], "w") as output:
                output.write("entered")
            time.sleep(60)
        result = {"content": [{"type": "text", "text": request["params"]["arguments"]["value"]}]}
    elif method == "ping":
        result = {}
    else:
        raise AssertionError(f"unexpected fixture method: {method}")
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
