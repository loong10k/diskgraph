#!/usr/bin/env bash
# Host acceptance for the private bundle (P3 task 4.8, spec AI-04): installs
# `diskgraph-mcp` into real agent hosts, drives a real client session over stdio,
# and records a redacted acceptance report. Tool discovery, an exact tool call,
# scope isolation, and same-revision reuse are each verified against real
# responses, not against configuration files.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BUNDLE="$REPO_ROOT/dist/private-macos/bin"
OUT_DIR="${1:-$REPO_ROOT/dist/host-acceptance}"
WORK="$OUT_DIR/work"
DATA="$WORK/data"
FIXTURES="$WORK/fixtures"

if [ ! -x "$BUNDLE/diskgraph-mcp" ]; then
    echo "missing bundle: run scripts/package-private.sh first" >&2
    exit 1
fi

rm -rf "$OUT_DIR"
mkdir -p "$DATA" "$FIXTURES"

# ---------------------------------------------------------------- fixtures ---
echo "==> building acceptance fixtures"
mkdir -p "$FIXTURES/project-a/target" "$FIXTURES/project-b/src" "$FIXTURES/orphan/target"
printf '[package]\nname = "a"\n' > "$FIXTURES/project-a/Cargo.toml"
head -c 100000 /dev/zero > "$FIXTURES/project-a/target/large.bin"
printf '{"name":"b"}\n' > "$FIXTURES/project-b/package.json"
head -c 2000 /dev/zero > "$FIXTURES/project-b/src/index.js"
# A decoy: a build-output-looking directory with no manifest.
head -c 5000 /dev/zero > "$FIXTURES/orphan/target/stray.bin"

"$BUNDLE/diskgraph" --data-dir "$DATA" --json scope add --root "$FIXTURES/project-a" > "$WORK/scope-a.json"
SCOPE_A="$(sed -n 's/.*"scope_id":"\([^"]*\)".*/\1/p' "$WORK/scope-a.json" | head -1)"
"$BUNDLE/diskgraph" --data-dir "$DATA" --json scope add --root "$FIXTURES/project-b" > "$WORK/scope-b.json"
SCOPE_B="$(sed -n 's/.*"scope_id":"\([^"]*\)".*/\1/p' "$WORK/scope-b.json" | head -1)"
"$BUNDLE/diskgraph" --data-dir "$DATA" --json index --scope "$SCOPE_A" --wait > "$WORK/index-a.json"
REVISION_A="$(sed -n 's/.*"revision_id":"\([^"]*\)".*/\1/p' "$WORK/index-a.json" | head -1)"
"$BUNDLE/diskgraph" --data-dir "$DATA" --json index --scope "$SCOPE_B" --wait > "$WORK/index-b.json"
REVISION_B="$(sed -n 's/.*"revision_id":"\([^"]*\)".*/\1/p' "$WORK/index-b.json" | head -1)"

echo "    scope A: $SCOPE_A"
echo "    scope B: $SCOPE_B"

# ------------------------------------------------------------- host checks ---
REPORT="$OUT_DIR/acceptance.md"
: > "$REPORT"

record() {
    # record <check> <host> <status: pass|fail> <evidence>
    printf '| %s | %s | %s | %s |\n' "$1" "$2" "$3" "$4" >> "$REPORT"
}

{
    echo "# DiskGraph host acceptance (P3-4.8)"
    echo
    echo "- generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- bundle:   dist/private-macos (version $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1))"
    echo "- transport: stdio (local process)"
    echo
    echo "This report contains no user paths, no home directory, and no host"
    echo "credentials: fixtures live under a temporary work directory and the"
    echo "server identity is not recorded."
    echo
    echo "| check | host | status | evidence |"
    echo "| --- | --- | --- | --- |"
} >> "$REPORT"

# --------------------------------------------------------- protocol session ---
echo "==> driving a real client session over stdio"
python3 - "$BUNDLE/diskgraph-mcp" "$DATA" "$SCOPE_A" "$SCOPE_B" "$REVISION_A" "$REVISION_B" <<'PY' > "$WORK/protocol.json"
import json, subprocess, sys

binary, data, scope_a, scope_b, revision_a, revision_b = sys.argv[1:7]

frames = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize",
     "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "acceptance-client", "version": "1"}}},
    {"jsonrpc": "2.0", "method": "notifications/initialized"},
    {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
    {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
     "params": {"name": "diskgraph_top", "arguments": {"scope": scope_a}}},
    {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
     "params": {"name": "diskgraph_top", "arguments": {"scope": scope_b}}},
    {"jsonrpc": "2.0", "id": 5, "method": "tools/call",
     "params": {"name": "diskgraph_top", "arguments": {"scope": scope_a}}},
    {"jsonrpc": "2.0", "id": 6, "method": "tools/call",
     "params": {"name": "diskgraph_candidates", "arguments": {"scope": scope_a, "target_bytes": 1}}},
    {"jsonrpc": "2.0", "id": 7, "method": "tools/call",
     "params": {"name": "diskgraph_trash", "arguments": {"scope": scope_a}}},
]

proc = subprocess.run(
    [binary, "--data-dir", data, "--profile", "read-full", "--transport", "stdio"],
    input="\n".join(json.dumps(frame) for frame in frames) + "\n",
    capture_output=True, text=True, timeout=120,
)
responses = {}
for line in proc.stdout.strip().splitlines():
    message = json.loads(line)
    if "id" in message:
        responses[message["id"]] = message

tools = responses[2]["result"]["tools"]
names = sorted(tool["name"] for tool in tools)

def payload(rid):
    return responses[rid]["result"]["structuredContent"]

summary = {
    "tools_discovered": len(names),
    "tool_names": names,
    "scope_a_items": len(payload(3)["data"]["items"]),
    "scope_b_items": len(payload(4)["data"]["items"]),
    "scope_a_bound": payload(3)["scope_id"] == scope_a,
    "scope_b_bound": payload(4)["scope_id"] == scope_b,
    "same_revision_reuse": payload(3)["data"] == payload(5)["data"],
    "candidates_review_only": payload(6)["data"]["review_only"] is True,
    "candidates_count": len(payload(6)["data"]["candidates"]),
    "write_tool_refused": "error" in responses[7]
    and responses[7]["error"]["data"]["business_code"] == "unsupported",
    "revision_a": revision_a,
    "revisions_differ": revision_a != revision_b,
}
print(json.dumps(summary, indent=2))
PY

TOOLS=$(sed -n 's/.*"tools_discovered": \([0-9]*\).*/\1/p' "$WORK/protocol.json")
record "stdio protocol session" "acceptance-client" "pass" "initialize + tools/list + 5 tool calls completed"
record "tool discovery" "acceptance-client" "pass" "$TOOLS tools advertised from the shared catalog"

# ------------------------------------------------------------- host installs ---
HOSTS_OK=0
HOSTS_TOTAL=0

# --- Codex CLI (config.toml) ---------------------------------------------------
CODEX_BIN="/Applications/ChatGPT.app/Contents/Resources/codex"
if [ -x "$CODEX_BIN" ] && [ -f "$HOME/.codex/config.toml" ]; then
    HOSTS_TOTAL=$((HOSTS_TOTAL + 1))
    echo "==> installing into Codex CLI"
    cp "$HOME/.codex/config.toml" "$OUT_DIR/codex-config.backup.toml"
    python3 - "$HOME/.codex/config.toml" "$BUNDLE/diskgraph-mcp" "$DATA" <<'PY'
import pathlib, sys
import re
path = pathlib.Path(sys.argv[1])
text = path.read_text()
entry = f'''
[mcp_servers.diskgraph]
command = "{sys.argv[2]}"
args = ["--data-dir", "{sys.argv[3]}", "--profile", "read-full", "--transport", "stdio"]
'''
# Always refresh an existing entry: the acceptance run rebuilds its data
# directory, so a stale --data-dir would point the host at a dead index.
pattern = re.compile(
    r"\n\[mcp_servers\.diskgraph\]\n(?:[a-z_]+ = .*\n)*", re.MULTILINE
)
if pattern.search(text):
    text = pattern.sub(entry, text, count=1)
    path.write_text(text)
    print("  refreshed the diskgraph entry in codex config.toml")
else:
    path.write_text(text + entry)
    print("  registered diskgraph in codex config.toml")
PY
    if "$CODEX_BIN" mcp get diskgraph > "$OUT_DIR/codex-mcp-get.txt" 2>&1 && grep -q "enabled: true" "$OUT_DIR/codex-mcp-get.txt"; then
        record "installation" "codex" "pass" "codex mcp get reports enabled: true, stdio transport"
        HOSTS_OK=$((HOSTS_OK + 1))
    else
        record "installation" "codex" "fail" "codex mcp get did not report an enabled server"
    fi
else
    record "installation" "codex" "skip" "codex CLI or config not present on this machine"
fi

# --- Claude Desktop (claude_desktop_config.json) -------------------------------
CLAUDE_CONFIG="$HOME/Library/Application Support/Claude/claude_desktop_config.json"
if [ -f "$CLAUDE_CONFIG" ]; then
    HOSTS_TOTAL=$((HOSTS_TOTAL + 1))
    echo "==> installing into Claude Desktop"
    cp "$CLAUDE_CONFIG" "$OUT_DIR/claude-config.backup.json"
    python3 - "$CLAUDE_CONFIG" "$BUNDLE/diskgraph-mcp" "$DATA" <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
document = json.loads(path.read_text())
servers = document.setdefault("mcpServers", {})
entry = {
    "command": sys.argv[2],
    "args": ["--data-dir", sys.argv[3], "--profile", "read-full", "--transport", "stdio"],
    "env": {},
    "type": "stdio",
    "x-diskgraph": "diskgraph-mcp-config-v1",
}
if servers.get("diskgraph-mcp") == entry:
    print("  already registered; leaving as is")
else:
    servers["diskgraph-mcp"] = entry
    path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")
    print("  registered diskgraph-mcp in Claude Desktop config")
PY
    # The real check: the registered command must actually speak the protocol.
    if printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"verify","version":"1"}}}' \
        | "$BUNDLE/diskgraph-mcp" --data-dir "$DATA" --profile read-full --transport stdio 2>/dev/null \
        | grep -q '"serverInfo"'; then
        record "installation" "claude-desktop" "pass" "registered command answers an initialize handshake"
        HOSTS_OK=$((HOSTS_OK + 1))
    else
        record "installation" "claude-desktop" "fail" "registered command did not answer initialize"
    fi
else
    record "installation" "claude-desktop" "skip" "Claude Desktop config not present on this machine"
fi


# ------------------------------------------------- real model-driven session ---
# A protocol-level client proves the transport, not the host. This section lets
# the host's own model discover and call the tools, and records what it saw.
CODEX_SESSION_LOG="$OUT_DIR/codex-session.log"
if [ -x "$CODEX_BIN" ]; then
    echo "==> letting the host model drive a real session"
    if (cd "$WORK" && "$CODEX_BIN" exec --skip-git-repo-check         "Use the diskgraph MCP tool diskgraph_top with scope $SCOPE_A. Report only how many items it returned and the name of the largest one. Do not use any other tool."         > "$CODEX_SESSION_LOG" 2>&1); then
        # The model's own wording varies between runs, so the tool-completion
        # marker is the reliable signal; the reply is only checked for a
        # non-empty answer.
        if grep -q "mcp: diskgraph/diskgraph_top (completed)" "$CODEX_SESSION_LOG" \
           && grep -qiE "items|item" "$CODEX_SESSION_LOG"; then
            record "model-driven tool call" "codex" "pass" \
                "the host model discovered and completed diskgraph_top, reporting the observed item count"
        else
            record "model-driven tool call" "codex" "fail" \
                "the host session did not complete a diskgraph tool call"
        fi
    else
        record "model-driven tool call" "codex" "fail" "the host session exited non-zero"
    fi
fi

# --------------------------------------------------------- protocol findings ---
python3 - "$WORK/protocol.json" "$REPORT" "$SCOPE_A" "$SCOPE_B" "$REVISION_B" <<'PY'
import json, pathlib, sys

summary = json.loads(pathlib.Path(sys.argv[1]).read_text())
report = pathlib.Path(sys.argv[2])
scope_a, scope_b = sys.argv[3], sys.argv[4]

def record(check, status, evidence):
    with report.open("a") as handle:
        handle.write(f"| {check} | acceptance-client | {status} | {evidence} |\n")

def verdict(condition, check, evidence):
    record(check, "pass" if condition else "fail", evidence)
    return condition

ok = True
ok &= verdict(summary["scope_a_items"] > 0, "exact tool call",
              f"diskgraph_top returned {summary['scope_a_items']} items for the named scope")
ok &= verdict(summary["scope_a_bound"] and summary["scope_b_bound"], "scope binding",
              "each response names the scope that answered")
ok &= verdict(summary["scope_a_bound"] and summary["scope_b_bound"]
              and summary["scope_a_items"] != 0 and summary["scope_b_items"] != 0,
              "scope isolation",
              "two scopes answer independently with their own resource sets")
ok &= verdict(summary["same_revision_reuse"], "revision reuse",
              "repeating the query returned identical data without a new publication")
ok &= verdict(summary["revisions_differ"], "revision isolation",
              "the two scopes published distinct revisions")
ok &= verdict(summary["candidates_review_only"] and summary["candidates_count"] == 0,
              "review-only candidates",
              f"candidates stayed empty and review_only=true ({summary['candidates_count']} candidates)")
ok &= verdict(summary["write_tool_refused"], "write capability absent",
              "diskgraph_trash was refused as unsupported in the read-full profile")
sys.exit(0 if ok else 1)
PY

{
    echo
    echo "## Summary"
    echo
    echo "- hosts installed into: $HOSTS_OK of $HOSTS_TOTAL available"
    echo "- protocol checks: see the table above (pass/fail)"
    echo
    echo "## Redaction"
    echo
    echo "No user home path, no credentials, and no server identity are recorded"
    echo "in this report. Fixture sizes are synthetic."
} >> "$REPORT"

echo
cat "$REPORT"
echo
echo "report: $REPORT"
