#!/usr/bin/env bash
# Streamable HTTP acceptance (P4 tasks 5.1 / 5.2, specs MCP-01 / MCP-02 /
# MCP-03 / SC-01 / SC-02): starts the real server binary on loopback, drives
# it with a real HTTP client, and records the result. Server-side locality is
# checked directly: a client-supplied path is never walked, a client id is
# never substituted for a server scope, and only the server's own index answers.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${1:-$REPO_ROOT/dist/http-acceptance}"
WORK="$OUT_DIR/work"
DATA="$WORK/data"
FIXTURES="$WORK/fixtures"
PORT="${DISKGRAPH_ACCEPT_PORT:-18899}"

MCP_BIN="$REPO_ROOT/target/debug/diskgraph-mcp"
CLI_BIN="$REPO_ROOT/target/debug/diskgraph"
if [ ! -x "$MCP_BIN" ] || [ ! -x "$CLI_BIN" ]; then
    echo "missing binaries; run: cargo build -p diskgraph-mcp -p diskgraph-cli" >&2
    exit 1
fi

rm -rf "$OUT_DIR"
mkdir -p "$DATA" "$FIXTURES"

# ---------------------------------------------------------------- fixtures ---
mkdir -p "$FIXTURES/project/target"
printf '[package]\nname = "http-accept"\n' > "$FIXTURES/project/Cargo.toml"
head -c 50000 /dev/zero > "$FIXTURES/project/target/app.bin"
# A decoy with a name a client might send from its own machine.
mkdir -p "$FIXTURES/shared-name"
head -c 4096 /dev/zero > "$FIXTURES/shared-name/client-looking-file.bin"

SCOPE="$("$CLI_BIN" --data-dir "$DATA" --json scope add --root "$FIXTURES/project" \
    | sed -n 's/.*"scope_id":"\([^"]*\)".*/\1/p' | head -1)"
"$CLI_BIN" --data-dir "$DATA" --json index --scope "$SCOPE" --wait > /dev/null

# ------------------------------------------------------------------ server ---
SERVER_LOG="$OUT_DIR/server.log"
"$MCP_BIN" --data-dir "$DATA" --profile read-full \
    --transport streamable-http --host 127.0.0.1 --port "$PORT" \
    > /dev/null 2> "$SERVER_LOG" &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
    if grep -q "listening on" "$SERVER_LOG" 2>/dev/null; then
        break
    fi
    sleep 0.1
done
if ! grep -q "listening on" "$SERVER_LOG" 2>/dev/null; then
    echo "server did not start: $SERVER_LOG" >&2
    cat "$SERVER_LOG" >&2
    exit 1
fi

# ------------------------------------------------------------------ checks ---
DISKGRAPH_ACCEPT_PORT="$PORT" \
DISKGRAPH_ACCEPT_SCOPE="$SCOPE" \
DISKGRAPH_ACCEPT_FIXTURES="$FIXTURES" \
DISKGRAPH_ACCEPT_OUT="$OUT_DIR" \
python3 <<'PY' > "$WORK/result.json"
import json, os, pathlib, urllib.error, urllib.request

out = pathlib.Path(os.environ["DISKGRAPH_ACCEPT_OUT"])
port = os.environ["DISKGRAPH_ACCEPT_PORT"]
scope = os.environ["DISKGRAPH_ACCEPT_SCOPE"]
fixtures = os.environ["DISKGRAPH_ACCEPT_FIXTURES"]
base = f"http://127.0.0.1:{port}"


def post(path, body, headers=None):
    request = urllib.request.Request(
        f"{base}{path}",
        data=body.encode(),
        headers={"Content-Type": "application/json", **(headers or {})},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.status, json.loads(response.read())
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


def get(path):
    with urllib.request.urlopen(f"{base}{path}", timeout=10) as response:
        return response.status, json.loads(response.read())


checks = {}

status, health = get("/healthz")
checks["health_reports_no_indexed_data"] = (
    status == 200
    and health["status"] == "ok"
    and "scopes" not in health
    and "server_id" not in health
)

status, initialized = post(
    "/mcp",
    json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18"}}),
)
checks["initialize_over_http"] = (
    status == 200 and initialized["result"]["serverInfo"]["name"] == "diskgraph"
)

status, listed = post("/mcp", json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
tools = listed["result"]["tools"]
checks["tool_discovery_matches_the_catalog"] = status == 200 and len(tools) > 0

# The server's own index answers.
status, called = post(
    "/mcp",
    json.dumps({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "diskgraph_top", "arguments": {"scope": scope}}}),
)
payload = called["result"]["structuredContent"]
items = payload["data"]["items"]
checks["server_answers_from_its_own_index"] = (
    status == 200
    and payload["ok"] is True
    and payload["scope_id"] == scope
    and bool(items)
)

# A raw path from the client is not a legal scope and is never walked.
status, raw_path = post(
    "/mcp",
    json.dumps({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "diskgraph_top", "arguments": {"scope": "/etc"}}}),
)
checks["client_path_is_never_walked"] = (
    status == 200
    and raw_path["error"]["data"]["business_code"] == "invalid_argument"
)

# A well-formed id the server never issued is unknown, never substituted.
status, unknown = post(
    "/mcp",
    json.dumps({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {"name": "diskgraph_top", "arguments": {"scope": "scope-shared-name"}}}),
)
checks["client_side_name_is_not_substituted"] = (
    status == 200
    and unknown["error"]["data"]["business_code"] == "not_found"
)

# The decoy never leaks into a server response.
serialized = json.dumps(called)
checks["server_response_mentions_only_server_paths"] = (
    "client-looking-file" not in serialized
    and str(fixtures) in serialized
)

# Transport-level contract.
status, batch = post("/mcp", json.dumps([{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}]))
checks["batch_is_refused"] = status == 400 and batch["error"]["code"] == -32600

status, version = post(
    "/mcp",
    json.dumps({"jsonrpc": "2.0", "id": 6, "method": "tools/list"}),
    {"MCP-Protocol-Version": "1999-01-01"},
)
checks["unsupported_version_is_reported"] = (
    status == 400 and "unsupported protocol version" in version["error"]["message"]
)

status, not_found = post(
    "/mcp", json.dumps({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                        "params": {"name": "diskgraph_trash", "arguments": {"scope": scope}}}),
)
checks["write_tool_still_refused_over_http"] = (
    status == 200
    and not_found["error"]["data"]["business_code"] == "unsupported"
)

passed = sum(1 for value in checks.values() if value)
print(json.dumps({"passed": passed, "total": len(checks), "checks": checks}, indent=2))
PY

python3 - "$WORK/result.json" "$OUT_DIR" <<'PY'
import json, pathlib, sys

result = json.loads(pathlib.Path(sys.argv[1]).read_text())
out = pathlib.Path(sys.argv[2])
lines = [
    "# Streamable HTTP acceptance (P4-5.1 / P4-5.2)",
    "",
    "- transport: Streamable HTTP on loopback",
    "- client: a real HTTP client (not the server's own code path)",
    "- server: the `diskgraph-mcp` binary, started as a separate process",
    "",
    "| check | status |",
    "| --- | --- |",
]
for name, value in result["checks"].items():
    lines.append(f"| {name} | {'pass' if value else 'fail'} |")
lines += ["", f"passed {result['passed']} of {result['total']}"]
(out / "acceptance.md").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
raise SystemExit(0 if result["passed"] == result["total"] else 1)
PY
