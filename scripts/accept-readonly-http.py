#!/usr/bin/env python3
"""Cross-platform real-binary acceptance for authenticated HTTP and legacy SSE."""

import base64
import csv
import contextlib
import hashlib
import hmac
import json
import os
import pathlib
import secrets
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


ROOT = pathlib.Path(__file__).resolve().parent.parent
SUFFIX = ".exe" if sys.platform == "win32" else ""
BIN_DIR = pathlib.Path(os.environ.get("DISKGRAPH_ACCEPT_BIN_DIR", ROOT / "target" / "debug"))
CLI = BIN_DIR / f"diskgraph{SUFFIX}"
MCP = BIN_DIR / f"diskgraph-mcp{SUFFIX}"
ISSUER, AUDIENCE, SUBJECT = "diskgraph-readonly-accept", "diskgraph", "remote-reader"
ORIGIN = "http://diskgraph-accept.invalid"


def restrict_windows_key(path):
    """Give the fixture only its owner, SYSTEM and Administrators access."""
    identity = subprocess.run(["whoami", "/user", "/fo", "csv", "/nh"],
                              capture_output=True, text=True, check=True)
    sid = next(csv.reader(line for line in identity.stdout.splitlines() if line.strip()))[1]
    subprocess.run(["icacls", str(path), "/inheritance:r"],
                   capture_output=True, text=True, check=True)
    subprocess.run(["icacls", str(path), "/grant:r", f"*{sid}:F",
                    "*S-1-5-18:F", "*S-1-5-32-544:F"],
                   capture_output=True, text=True, check=True)


def token(key, subject=SUBJECT):
    def segment(value):
        data = json.dumps(value, separators=(",", ":")).encode()
        return base64.urlsafe_b64encode(data).rstrip(b"=").decode()

    message = segment({"alg": "HS256", "typ": "JWT"}) + "." + segment({
        "iss": ISSUER, "aud": AUDIENCE, "sub": subject,
        "exp": int(time.time()) + 600, "scope": "metadata:read",
    })
    signature = base64.urlsafe_b64encode(
        hmac.new(key.encode(), message.encode(), hashlib.sha256).digest()
    ).rstrip(b"=").decode()
    return message + "." + signature


def principal():
    digest = hashlib.sha256()
    issuer = ISSUER.encode()
    digest.update(len(issuer).to_bytes(8, "big"))
    digest.update(issuer)
    digest.update(SUBJECT.encode())
    return ("subject-" + digest.hexdigest())[:64]


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


@contextlib.contextmanager
def server(data, transport, key_file, work):
    port = free_port()
    log_path = work / f"{transport}.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen([
            MCP, "--data-dir", data, "--profile", "read-full",
            "--auth-key-file", ISSUER, AUDIENCE, key_file, "--allowed-origin", ORIGIN,
            "--transport", transport, "--host", "127.0.0.1", "--port", str(port),
        ], stdout=subprocess.DEVNULL, stderr=log)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise RuntimeError(f"server exited early: {log_path.read_text()}")
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.05)
            else:
                raise RuntimeError(f"server did not listen: {log_path.read_text()}")
            yield port
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


def request(port, path, body=None, bearer=None, origin=None):
    headers = {}
    if bearer:
        headers["Authorization"] = f"Bearer {bearer}"
    if origin:
        headers["Origin"] = origin
    if body is not None:
        headers["Content-Type"] = "application/json"
    wire = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=wire, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        raw = error.read()
        return error.code, json.loads(raw) if raw else None


class LineReader:
    def __init__(self, stream):
        self.stream, self.buffer = stream, b""

    def readline(self):
        while b"\n" not in self.buffer:
            chunk = self.stream.recv(65536)
            if not chunk:
                raise ConnectionError("SSE stream ended")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return line.decode().rstrip("\r")


def event(reader):
    kind, data = "message", ""
    while True:
        line = reader.readline()
        if not line:
            if data:
                return kind, data
            continue
        if line.startswith("event:"):
            kind = line[6:].strip()
        elif line.startswith("data:"):
            data += line[5:].lstrip()


def raw_get(port, path, bearer=None, origin=None, keep_open=False):
    stream = socket.create_connection(("127.0.0.1", port), timeout=10)
    stream.settimeout(10)
    headers = f"GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n"
    if bearer:
        headers += f"Authorization: Bearer {bearer}\r\n"
    if origin:
        headers += f"Origin: {origin}\r\n"
    stream.sendall((headers + "Connection: close\r\n\r\n").encode())
    reader = LineReader(stream)
    status = int(reader.readline().split()[1])
    while reader.readline():
        pass
    if not keep_open:
        stream.close()
    return status, stream, reader


def raw_post(port, path, payload, bearer):
    body = json.dumps(payload).encode()
    with socket.create_connection(("127.0.0.1", port), timeout=10) as stream:
        headers = (
            f"POST {path} HTTP/1.1\r\nHost: localhost\r\n"
            f"Authorization: Bearer {bearer}\r\nOrigin: {ORIGIN}\r\n"
            f"Content-Type: application/json\r\nContent-Length: {len(body)}\r\n"
            "Connection: close\r\n\r\n"
        )
        stream.sendall(headers.encode() + body)
        return int(LineReader(stream).readline().split()[1])


def main():
    checks = {}
    with tempfile.TemporaryDirectory(prefix="diskgraph-http-accept-") as temporary:
        work = pathlib.Path(temporary)
        data = work / "data"
        root = work / "project"
        (root / "target").mkdir(parents=True)
        (root / "Cargo.toml").write_text("[package]\nname='accept'\n")
        (root / "target" / "bin").write_bytes(bytes(4096))
        added = subprocess.run([CLI, "--data-dir", data, "--json", "scope", "add", "--root", root],
                               capture_output=True, text=True, check=True, timeout=120)
        scope = json.loads(added.stdout)["data"]["scope_id"]
        subprocess.run([CLI, "--data-dir", data, "--json", "index", "--scope", scope, "--wait"],
                       capture_output=True, text=True, check=True, timeout=120)
        with contextlib.closing(sqlite3.connect(data / "diskgraph-control.sqlite")) as connection, connection:
            version = connection.execute("SELECT version FROM policy WHERE id = 1").fetchone()[0]
            connection.execute("INSERT INTO grants (principal_id, permission, scope_id, policy_version) "
                               "VALUES (?, ?, ?, ?)", (principal(), "metadata:read", scope, version))
        key = secrets.token_urlsafe(32)
        key_file = work / "auth.key"
        key_file.write_text(key)
        key_file.chmod(0o600)
        if sys.platform == "win32":
            restrict_windows_key(key_file)
        bearer = token(key)
        top = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
            "name": "diskgraph_top", "arguments": {"scope": scope},
        }}

        with server(data, "streamable-http", key_file, work) as port:
            anonymous, _ = request(port, "/mcp", top)
            hostile, _ = request(port, "/mcp", top, bearer, "http://evil.invalid")
            ungranted, ungranted_answer = request(port, "/mcp", top, token(key, "ungranted"), ORIGIN)
            status, answer = request(port, "/mcp", top, bearer, ORIGIN)
            checks["modern_anonymous_401"] = anonymous == 401
            checks["modern_hostile_origin_403"] = hostile == 403
            checks["modern_signed_but_ungranted_denied"] = (ungranted == 200 and
                ungranted_answer["error"]["data"]["business_code"] == "permission_denied")
            checks["modern_authorized_scope_read"] = (status == 200 and
                answer["result"]["structuredContent"]["scope_id"] == scope and
                bool(answer["result"]["structuredContent"]["data"]["items"]))
            with contextlib.closing(sqlite3.connect(data / "diskgraph-control.sqlite")) as connection, connection:
                connection.execute("DELETE FROM grants WHERE principal_id = ?", (principal(),))
            revoked, revoked_answer = request(port, "/mcp", top, bearer, ORIGIN)
            checks["modern_live_revocation_denied"] = (revoked == 200 and
                revoked_answer["error"]["data"]["business_code"] == "permission_denied")

        with contextlib.closing(sqlite3.connect(data / "diskgraph-control.sqlite")) as connection, connection:
            connection.execute("INSERT INTO grants (principal_id, permission, scope_id, policy_version) "
                               "VALUES (?, ?, ?, ?)", (principal(), "metadata:read", scope, version))

        with server(data, "legacy-sse", key_file, work) as port:
            anonymous, _, _ = raw_get(port, "/sse")
            hostile, _, _ = raw_get(port, "/sse", bearer, "http://evil.invalid")
            status, stream, reader = raw_get(port, "/sse", bearer, ORIGIN, keep_open=True)
            try:
                kind, endpoint = event(reader)
                accepted = raw_post(port, endpoint, top, bearer)
                response_kind, payload = event(reader)
                response = json.loads(payload)
            finally:
                stream.close()
            checks["legacy_anonymous_401"] = anonymous == 401
            checks["legacy_hostile_origin_403"] = hostile == 403
            checks["legacy_endpoint_event"] = status == 200 and kind == "endpoint" and endpoint.startswith("/messages/?session_id=")
            checks["legacy_authorized_scope_read"] = (accepted == 202 and response_kind == "message" and
                response["result"]["structuredContent"]["scope_id"] == scope)

        def rejects_key(path):
            result = subprocess.run([
                MCP, "--data-dir", data, "--transport", "streamable-http",
                "--auth-key-file", ISSUER, AUDIENCE, path,
            ], capture_output=True, text=True, timeout=10)
            return result.returncode == 6 and key not in result.stdout + result.stderr

        checks["missing_key_file_refused"] = rejects_key(work / "missing.key")
        short_file = work / "short.key"
        short_file.write_text("tiny")
        short_file.chmod(0o600)
        checks["short_key_file_refused"] = rejects_key(short_file)
        forwarded = subprocess.run([
            CLI, "--data-dir", data, "serve", "--transport", "streamable-http",
            "--auth-key-file", ISSUER, AUDIENCE, short_file,
        ], capture_output=True, text=True, timeout=10)
        checks["cli_serve_forwards_key_file"] = forwarded.returncode == 6
        if sys.platform != "win32":
            key_file.chmod(0o644)
            checks["group_readable_key_refused"] = rejects_key(key_file)
        else:
            subprocess.run(["icacls", str(key_file), "/grant", "*S-1-1-0:R"],
                           capture_output=True, text=True, check=True)
            checks["everyone_readable_key_refused"] = rejects_key(key_file)

    passed = sum(checks.values())
    print(json.dumps({"passed": passed, "total": len(checks), "checks": checks}, indent=2))
    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
