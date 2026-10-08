#!/usr/bin/env python3
"""Cross-platform real-binary acceptance for authenticated HTTP and legacy SSE."""

import argparse
import base64
from collections import deque
import csv
import contextlib
import hashlib
import hmac
import json
import math
import os
import pathlib
import secrets
import socket
import sqlite3
import stat
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


def binary_evidence(path):
    """有限读取实际镜像内容；只记录摘要，不将其当作执行身份或授权。"""
    maximum = 128 * 1024 * 1024
    digest = hashlib.sha256()
    length = 0
    with pathlib.Path(path).open('rb') as image:
        before = os.fstat(image.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > maximum:
            raise RuntimeError('acceptance image is not a bounded regular file')
        while chunk := image.read(min(1024 * 1024, maximum - length + 1)):
            length += len(chunk)
            if length > maximum:
                raise RuntimeError('acceptance image exceeded byte budget')
            digest.update(chunk)
        after = os.fstat(image.fileno())
        fields = ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')
        if length != before.st_size or any(getattr(before, key) != getattr(after, key) for key in fields):
            raise RuntimeError('acceptance image changed during evidence capture')
    return {'bytes': length, 'sha256': digest.hexdigest()}


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


def stop_server(process):
    """停止原进程并实际等待；不能将强制清理当作正常退出验收。"""
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
        raise RuntimeError('normal shutdown exceeded 5 seconds; forced cleanup is not acceptance')
    if sys.platform != 'win32' and process.returncode != 0:
        raise RuntimeError(f'normal shutdown failed: original process exited {process.returncode}')


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
                    acl = ""
                    if sys.platform == "win32":
                        acl = subprocess.run(["icacls", str(key_file)], capture_output=True,
                                             text=True).stdout
                    raise RuntimeError(f"server exited early: {log_path.read_text()}\nkey ACL: {acl}")
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.05)
            else:
                raise RuntimeError(f"server did not listen: {log_path.read_text()}")
            yield port
        finally:
            stop_server(process)


def request(port, path, body=None, bearer=None, origin=None, *, timeout=10):
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
        with urllib.request.urlopen(req, timeout=timeout) as response:
            raw = read_response(response)
            return response.status, json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        with error:
            raw = read_response(error)
            return error.code, json.loads(raw) if raw else None


def read_response(response):
    """响应上限为1MiB；超限拒绝，不持有无限响应或输出正文。"""
    limit = 1024 * 1024
    raw = response.read(limit + 1)
    if len(raw) > limit:
        raise RuntimeError('HTTP acceptance response exceeds 1 MiB')
    return raw


def soak_reads(port, body, key, scope, seconds):
    """在原截止时间内重复实际请求；有限采样不自动证明长期生产稳定性。"""
    if not math.isfinite(seconds) or not 0 < seconds <= 86400:
        raise ValueError('soak duration must be finite and in (0, 86400] seconds')
    started = time.monotonic()
    deadline = started + seconds
    samples = deque(maxlen=4096)
    count = 0
    # 默认服务桶为每客户端50次/秒；持续稳定性负载留出控制请求余量。
    # 按实际开始时间节流，慢请求后不补发积压；429仍失败，绝不重试。
    offered_rate = 40
    next_start = started
    while (remaining := deadline - time.monotonic()) > 0:
        delay = next_start - time.monotonic()
        if delay > 0:
            time.sleep(min(delay, remaining))
            continue
        before = time.monotonic()
        remaining = deadline - before
        if remaining <= 0:
            break
        next_start = before + 1 / offered_rate
        # 为后续请求签发同一主体的新token，不更新总期限或数据库授权。
        bearer = token(key)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        try:
            status, answer = request(port, '/mcp', body, bearer, ORIGIN,
                                     timeout=min(10, remaining))
        except OSError as error:
            # 原异常与截止时间保持；只增加数值阶段信息，区分末段期限和服务停顿。
            error.add_note('soak_request_timing ' + json.dumps({
                'completed_requests': count, 'remaining_at_start_ms': remaining * 1000,
                'request_elapsed_ms': (time.monotonic() - before) * 1000,
                'elapsed_seconds': time.monotonic() - started,
                'offered_max_requests_per_second': offered_rate,
            }, sort_keys=True))
            raise
        result = answer.get('result') if isinstance(answer, dict) else None
        content = result.get('structuredContent') if isinstance(result, dict) else None
        data = content.get('data') if isinstance(content, dict) else None
        scope_matches = isinstance(content, dict) and content.get('scope_id') == scope
        items_present = (isinstance(data, dict) and isinstance(data.get('items'), list)
                         and bool(data['items']))
        rpc_error = isinstance(answer, dict) and 'error' in answer
        if status != 200 or rpc_error or not scope_matches or not items_present:
            # 只保留允许列表分类，原正文、路径、主体和错误message绝不进入诊断。
            error = answer.get('error') if isinstance(answer, dict) else None
            error_data = error.get('data') if isinstance(error, dict) else None
            code = error_data.get('business_code') if isinstance(error_data, dict) else None
            allowed = {'invalid_argument', 'ambiguous', 'permission_denied', 'approval_required',
                       'not_indexed', 'not_found', 'stale_plan', 'revision_expired',
                       'incompatible_history', 'unsupported', 'unavailable', 'budget_exceeded',
                       'timeout', 'resource_exhausted', 'partial', 'needs_attention',
                       'recovery_unconfirmed', 'conflict', 'idempotency_conflict', 'internal_error'}
            diagnostic = {'http_status': status if type(status) is int and 100 <= status <= 599 else None,
                          'business_code': code if isinstance(code, str) and code in allowed else 'unknown',
                          'rpc_error': rpc_error,
                          'scope_matches': scope_matches, 'items_present': items_present}
            raise RuntimeError('soak read failed ' + json.dumps(diagnostic, sort_keys=True))
        samples.append((time.monotonic() - before) * 1000)
        count += 1
    if not count:
        raise RuntimeError('soak read completed no requests')
    ordered = sorted(samples)
    return {'requests': count, 'requested_seconds': seconds,
            'elapsed_seconds': time.monotonic() - started,
            'offered_max_requests_per_second': offered_rate,
            'latency_sample_window': 'last_4096_requests', 'latency_samples': len(ordered),
            'p50_ms': ordered[math.ceil(len(ordered) * 0.50) - 1],
            'p95_ms': ordered[math.ceil(len(ordered) * 0.95) - 1],
            'native_long_run_qualified': False}


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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--soak-seconds', type=float, default=0,
                        help='Optional sustained HTTP reads; 0 disables, maximum 86400 seconds')
    parser.add_argument('--output', type=pathlib.Path,
                        help='Save the completed acceptance report at this path')
    args = parser.parse_args()
    if not math.isfinite(args.soak_seconds) or not 0 <= args.soak_seconds <= 86400:
        parser.error('--soak-seconds must be finite and between 0 and 86400')
    if args.output and args.output.exists():
        parser.error('--output already exists; use a fresh report path')
    binary_before = {name: binary_evidence(path) for name, path in [('cli', CLI), ('mcp', MCP)]}
    checks = {}
    soak = None
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
            if args.soak_seconds:
                soak = soak_reads(port, top, key, scope, args.soak_seconds)
                checks['modern_sustained_scope_reads'] = soak['requests'] > 0
                # 长时运行后使用仍有效的同主体token，撤权检查不能被token到期替代。
                bearer = token(key)
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
    binary_after = {name: binary_evidence(path) for name, path in [('cli', CLI), ('mcp', MCP)]}
    if binary_before != binary_after:
        raise RuntimeError('acceptance CLI or MCP image changed between phases')
    report = json.dumps({"passed": passed, "total": len(checks), "checks": checks,
                         "sustained_http": soak, "platform": sys.platform,
                         "binary_content_before": binary_before,
                         "binary_content_after": binary_after}, indent=2)
    if args.output:
        # 独占创建避免并发验收覆盖旧证据；中途失败不伪造完成报告。
        with args.output.open('x', encoding='utf-8') as output:
            output.write(report + '\n')
    print(report)
    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
