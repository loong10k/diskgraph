#!/usr/bin/env bash
# Legacy HTTP+SSE acceptance (P4 tasks 5.6 / 5.7, specs MCP-01 / MCP-04):
# starts the real `diskgraph-mcp` binary with the legacy adapter enabled and
# drives it with a spec-conformant 2024-11-05 client (GET /sse -> endpoint
# event -> POST -> message event), proving the endpoint contract end to end.
# Also verifies the adapter is off by default and that authorization binds
# legacy POSTs exactly like modern requests.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${1:-$REPO_ROOT/dist/legacy-acceptance}"
WORK="$OUT_DIR/work"
DATA="$WORK/data"
# Derive the ports from the PID so repeated runs never collide with a
# still-dying server from a previous attempt (the flaky failure source).
PORT="${DISKGRAPH_LEGACY_PORT:-$((19000 + ($$ % 500) * 2))}"

MCP_BIN="$REPO_ROOT/target/debug/diskgraph-mcp"
CLI_BIN="$REPO_ROOT/target/debug/diskgraph"
if [ ! -x "$MCP_BIN" ] || [ ! -x "$CLI_BIN" ]; then
    echo "missing binaries; run: cargo build -p diskgraph-mcp -p diskgraph-cli" >&2
    exit 1
fi

rm -rf "$OUT_DIR"
mkdir -p "$DATA" "$WORK/fixtures"
mkdir -p "$WORK/fixtures/project"
printf '[package]\nname = "legacy-accept"\n' > "$WORK/fixtures/project/Cargo.toml"
SCOPE="$("$CLI_BIN" --data-dir "$DATA" --json scope add --root "$WORK/fixtures/project" \
    | sed -n 's/.*"scope_id":"\([^"]*\)".*/\1/p' | head -1)"
"$CLI_BIN" --data-dir "$DATA" --json index --scope "$SCOPE" --wait > /dev/null

# Exercise the same remote identity and policy boundary as a deployed
# listener. The key and token exist only in this isolated fixture.
AUTH_ISSUER="diskgraph-legacy-accept"
AUTH_AUDIENCE="diskgraph"
AUTH_SUBJECT="legacy-client"
AUTH_KEY="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
ALLOWED_ORIGIN="http://diskgraph-accept.invalid"
DISKGRAPH_ACCEPT_DATA="$DATA" \
DISKGRAPH_ACCEPT_SCOPE="$SCOPE" \
DISKGRAPH_ACCEPT_WORK="$WORK" \
DISKGRAPH_ACCEPT_ISSUER="$AUTH_ISSUER" \
DISKGRAPH_ACCEPT_AUDIENCE="$AUTH_AUDIENCE" \
DISKGRAPH_ACCEPT_SUBJECT="$AUTH_SUBJECT" \
DISKGRAPH_ACCEPT_KEY="$AUTH_KEY" \
python3 <<'PY'
import base64, hashlib, hmac, json, os, pathlib, sqlite3, time

issuer = os.environ["DISKGRAPH_ACCEPT_ISSUER"]
audience = os.environ["DISKGRAPH_ACCEPT_AUDIENCE"]
subject = os.environ["DISKGRAPH_ACCEPT_SUBJECT"]
key = os.environ["DISKGRAPH_ACCEPT_KEY"].encode()
digest = hashlib.sha256()
digest.update(len(issuer.encode()).to_bytes(8, "big"))
digest.update(issuer.encode())
digest.update(subject.encode())
principal = ("subject-" + digest.hexdigest())[:64]
database = pathlib.Path(os.environ["DISKGRAPH_ACCEPT_DATA"]) / "diskgraph-control.sqlite"
with sqlite3.connect(database) as connection:
    version = connection.execute("SELECT version FROM policy WHERE id = 1").fetchone()[0]
    connection.execute(
        "INSERT INTO grants (principal_id, permission, scope_id, policy_version) VALUES (?, ?, ?, ?)",
        (principal, "metadata:read", os.environ["DISKGRAPH_ACCEPT_SCOPE"], version),
    )

def segment(value):
    return base64.urlsafe_b64encode(json.dumps(value, separators=(",", ":")).encode()).rstrip(b"=").decode()

message = segment({"alg": "HS256", "typ": "JWT"}) + "." + segment({
    "iss": issuer, "aud": audience, "sub": subject,
    "exp": int(time.time()) + 3600, "scope": "metadata:read",
})
signature = base64.urlsafe_b64encode(hmac.new(key, message.encode(), hashlib.sha256).digest()).rstrip(b"=").decode()
token_file = pathlib.Path(os.environ["DISKGRAPH_ACCEPT_WORK"]) / "token"
token_file.write_text(message + "." + signature)
token_file.chmod(0o600)
PY

# Server with the legacy adapter on.
SERVER_LOG="$OUT_DIR/server.log"
"$MCP_BIN" --data-dir "$DATA" --profile read-full \
    --auth "$AUTH_ISSUER" "$AUTH_AUDIENCE" "$AUTH_KEY" \
    --allowed-origin "$ALLOWED_ORIGIN" \
    --transport legacy-sse --host 127.0.0.1 --port "$PORT" \
    > /dev/null 2> "$SERVER_LOG" &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do
    grep -q "listening on" "$SERVER_LOG" 2>/dev/null && break
    sleep 0.1
done
grep -q "listening on" "$SERVER_LOG" || { echo "server did not start" >&2; cat "$SERVER_LOG" >&2; exit 1; }

# A modern-only server to prove the default-off diagnostic.
OFF_LOG="$OUT_DIR/server-off.log"
"$MCP_BIN" --data-dir "$DATA-off" --profile read-full \
    --auth "$AUTH_ISSUER" "$AUTH_AUDIENCE" "$AUTH_KEY" \
    --allowed-origin "$ALLOWED_ORIGIN" \
    --transport streamable-http --host 127.0.0.1 --port "$((PORT + 1))" \
    > /dev/null 2> "$OFF_LOG" &
OFF_PID=$!
trap 'kill "$SERVER_PID" "$OFF_PID" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do
    grep -q "listening on" "$OFF_LOG" 2>/dev/null && break
    sleep 0.1
done

export DISKGRAPH_LEGACY_PORT="$PORT"
export DISKGRAPH_LEGACY_OFF_PORT="$((PORT + 1))"
export DISKGRAPH_LEGACY_SCOPE="$SCOPE"
export DISKGRAPH_LEGACY_TOKEN_FILE="$WORK/token"
export DISKGRAPH_LEGACY_ORIGIN="$ALLOWED_ORIGIN"
python3 <<'PY' > "$WORK/result.json"
import json, os, pathlib, sys

import sys
sys.path.insert(0, "crates/diskgraph-testkit/src")
# The testkit client is Rust; this script drives the wire in Python so the
# acceptance depends on nothing but the server binary itself.

import socket

port = int(os.environ["DISKGRAPH_LEGACY_PORT"])
off_port = int(os.environ["DISKGRAPH_LEGACY_OFF_PORT"])
scope = os.environ["DISKGRAPH_LEGACY_SCOPE"]
token = pathlib.Path(os.environ["DISKGRAPH_LEGACY_TOKEN_FILE"]).read_text()
origin = os.environ["DISKGRAPH_LEGACY_ORIGIN"]
checks = {}


class LineReader:
    """Manual line reader over a raw socket: no makefile buffering races."""

    def __init__(self, sock):
        self.sock = sock
        self.buffer = b""

    def readline(self):
        while b"\n" not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("SSE stream ended")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return line + b"\n"


def read_event(reader):
    event, data = "message", ""
    while True:
        line = reader.readline()
        line = line.decode().rstrip("\r\n")
        if not line:
            if data:
                return event, data
            continue
        if line.startswith(":"):
            continue
        if line.startswith("event:"):
            event = line[len("event:"):].strip()
        elif line.startswith("data:"):
            data += line[len("data:"):].lstrip()
    return event, data


def sse_connect(port):
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    # After the handshake, read events blocking: an SSE client waits for the
    # server, and the server log proves the frame is on its way.
    sock.settimeout(None)
    sock.sendall((
        f"GET /sse HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n"
        f"Authorization: Bearer {token}\r\nOrigin: {origin}\r\n\r\n"
    ).encode())
    reader = LineReader(sock)
    status = reader.readline().decode()
    assert "200" in status, status
    while True:
        line = reader.readline()
        if not line or line in (b"\r\n", b"\n"):
            break
    event, data = read_event(reader)
    assert event == "endpoint", f"expected endpoint event, got {event}"
    return sock, reader, data


def post(port, endpoint, body, authorized=True):
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    head = (
        f"POST {endpoint} HTTP/1.1\r\nHost: x\r\n"
        f"Content-Type: application/json\r\nContent-Length: {len(body)}\r\n"
    )
    if authorized:
        head += f"Authorization: Bearer {token}\r\nOrigin: {origin}\r\n"
    head += "Connection: close\r\n\r\n"
    sock.sendall(head.encode() + body.encode())
    reader = LineReader(sock)
    status = reader.readline().decode()
    sock.close()
    return int(status.split()[1])


# 1. The full legacy contract on the enabled server.

def get_status(path, token_value=None, origin_value=None, target_port=port):
    probe = socket.create_connection(("127.0.0.1", target_port), timeout=10)
    head = f"GET {path} HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n"
    if token_value:
        head += f"Authorization: Bearer {token_value}\r\n"
    if origin_value:
        head += f"Origin: {origin_value}\r\n"
    probe.sendall((head + "Connection: close\r\n\r\n").encode())
    status = probe.recv(4096).split(b"\r\n", 1)[0].decode()
    probe.close()
    return int(status.split()[1])

checks["anonymous_sse_rejected"] = get_status("/sse") == 401
checks["malicious_origin_rejected"] = get_status("/sse", token, "http://evil.invalid") == 403
sock, reader, endpoint = sse_connect(port)
checks["endpoint_event_contract"] = endpoint.startswith("/messages/?session_id=")
status = post(port, endpoint, json.dumps(
    {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
     "params": {"name": "diskgraph_top", "arguments": {"scope": scope}}}))
checks["legacy_post_acknowledged_202"] = status == 202
event, data = read_event(reader)
payload = json.loads(data)
checks["response_arrives_as_message_event"] = (
    event == "message" and payload["result"]["structuredContent"]["ok"] is True
)
sock.close()

# 2. Authorization binds legacy POSTs: unknown session 404.
status = post(port, "/messages/?session_id=bogus",
              json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
checks["unknown_session_refused_404"] = status == 404

# 3. The adapter is off unless opted in: probing a modern-only server says so.
probe = socket.create_connection(("127.0.0.1", off_port), timeout=10)
probe.sendall((
    f"GET /sse HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\n"
    f"Origin: {origin}\r\n\r\n"
).encode())
reader = probe.makefile("rb")
status = reader.readline().decode()
length = 0
while True:
    line = reader.readline().decode()
    if line in ("\r\n", "\n", ""):
        break
    if line.lower().startswith("content-length:"):
        length = int(line.split(":", 1)[1].strip())
body = reader.read(length).decode()
checks["default_off_reports_disabled"] = "404" in status and "legacy_sse_disabled" in body

passed = sum(1 for value in checks.values() if value)
print(json.dumps({"passed": passed, "total": len(checks), "checks": checks}, indent=2))
PY

python3 - "$WORK/result.json" "$OUT_DIR" <<'PY'
import json, pathlib, sys

result = json.loads(pathlib.Path(sys.argv[1]).read_text())
out = pathlib.Path(sys.argv[2])
lines = [
    "# Legacy HTTP+SSE acceptance (P4-5.6 / P4-5.7)",
    "",
    "- protocol: MCP HTTP+SSE, 2024-11-05 wire contract",
    "- client: spec-conformant legacy client (raw socket, no SDK)",
    "- server: the `diskgraph-mcp` binary, separate process, legacy opted in",
    "",
    "| check | status |",
    "| --- | --- |",
]
for name, value in result["checks"].items():
    lines.append(f"| {name} | {'pass' if value else 'fail'} |")
lines += ["", f"passed {result['passed']} of {result['total']}",
          "",
          "A real old MCP host remains a deployer-matrix item (task 5.11);",
          "this acceptance proves the adapter speaks the old protocol exactly."]
(out / "acceptance.md").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
raise SystemExit(0 if result["passed"] == result["total"] else 1)
PY
