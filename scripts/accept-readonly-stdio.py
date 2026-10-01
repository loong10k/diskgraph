#!/usr/bin/env python3
"""Portable, isolated CLI and real-process stdio MCP acceptance."""

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import tomllib


ROOT = pathlib.Path(__file__).resolve().parent.parent
SUFFIX = ".exe" if sys.platform == "win32" else ""
BIN_DIR = pathlib.Path(os.environ.get("DISKGRAPH_ACCEPT_BIN_DIR", ROOT / "target" / "debug"))
CLI = BIN_DIR / f"diskgraph{SUFFIX}"
MCP = BIN_DIR / f"diskgraph-mcp{SUFFIX}"


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=120, check=True)
    return [json.loads(line) for line in result.stdout.splitlines() if line.strip()]


def require(name, condition, checks):
    checks[name] = bool(condition)


def main():
    checks = {}
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    for name, binary in (("diskgraph", CLI), ("diskgraph-mcp", MCP)):
        observed = subprocess.run([binary, "--version"], capture_output=True, text=True, timeout=10)
        require(f"{name}_version", observed.returncode == 0 and
                observed.stdout.strip() == f"{name} {version}", checks)
    with tempfile.TemporaryDirectory(prefix="diskgraph-readonly-accept-") as temporary:
        work = pathlib.Path(temporary)
        data = work / "data"
        roots = [work / "first", work / "second"]
        scopes = []
        revisions = []
        for number, root in enumerate(roots):
            (root / "target").mkdir(parents=True)
            (root / "Cargo.toml").write_text(f"[package]\nname='accept-{number}'\n")
            (root / "target" / "bin").write_bytes(bytes(4096 + number))
            added = run(CLI, "--data-dir", data, "--json", "scope", "add", "--root", root)
            scope = added[-1]["data"]["scope_id"]
            scopes.append(scope)
            indexed = run(CLI, "--data-dir", data, "--json", "index", "--scope", scope, "--wait")
            revisions.append(indexed[-1]["data"]["revision_id"])

        def cli(*args):
            return run(CLI, "--data-dir", data, "--json", *args)[-1]

        first_tree = cli("tree", "--scope", scopes[0], "--depth", "2")
        second_tree = cli("tree", "--scope", scopes[1], "--depth", "2")
        first_top = cli("top", "--scope", scopes[0])
        candidates = cli("candidates", "--scope", scopes[0], "--target-bytes", "1")
        require("cli_tree_revision_binding", first_tree["data"]["revision_id"] == revisions[0]
                and second_tree["data"]["revision_id"] == revisions[1]
                and revisions[0] != revisions[1], checks)
        require("cli_top_bounded", first_top["ok"] and bool(first_top["data"]["items"]), checks)
        require("cli_candidates_review_only", candidates["data"]["review_only"] and candidates["data"]["remaining_bytes"] == "1", checks)

        frames = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            *(
                {"jsonrpc": "2.0", "id": 3 + index, "method": "tools/call", "params": {
                    "name": "diskgraph_top", "arguments": {"scope": scope},
                }}
                for index, scope in enumerate(scopes)
            ),
            {"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {
                "name": "diskgraph_trash", "arguments": {"scope": scopes[0]},
            }},
        ]
        session = subprocess.run(
            [MCP, "--data-dir", data, "--profile", "read-full", "--transport", "stdio"],
            input="\n".join(json.dumps(frame) for frame in frames) + "\n",
            capture_output=True, text=True, timeout=120, check=True,
        )
        responses = {message["id"]: message for line in session.stdout.splitlines()
                     if (message := json.loads(line)).get("id") is not None}
        require("stdio_initialize", responses[1]["result"]["serverInfo"]["name"] == "diskgraph", checks)
        require("stdio_tool_discovery", len(responses[2]["result"]["tools"]) > 0, checks)
        for index, scope in enumerate(scopes):
            result = responses[3 + index]["result"]["structuredContent"]
            require(f"stdio_scope_{index}_bound", result["ok"] and result["scope_id"] == scope, checks)
        require("stdio_write_tool_disabled", responses[5]["error"]["data"]["business_code"] == "unsupported", checks)

        forwarded = subprocess.run(
            [CLI, "--data-dir", data, "serve", "--profile", "read-full", "--transport", "stdio"],
            input=json.dumps(frames[0]) + "\n", capture_output=True, text=True, timeout=120,
        )
        reply = json.loads(forwarded.stdout.splitlines()[0]) if forwarded.stdout.strip() else {}
        require("cli_serve_delegates_to_mcp_binary", forwarded.returncode == 0 and
                reply.get("result", {}).get("serverInfo", {}).get("name") == "diskgraph", checks)

    passed = sum(checks.values())
    print(json.dumps({"passed": passed, "total": len(checks), "checks": checks}, indent=2))
    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
