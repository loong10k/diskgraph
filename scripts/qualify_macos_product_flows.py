"""普通 UID 的真实 CLI/MCP 产品闭环；仅由受保护安装资格驱动调用。"""
import hashlib
import json
import os
import select
import subprocess
import time
from pathlib import Path


def envelope(value):
    """只接受真实 v2 成功对象；协议/业务错误不能变成产品通过。"""
    if value.get("api_version") != 2 or value.get("ok") is not True or value.get("error"):
        raise RuntimeError("product returned a failed or incompatible envelope")
    if not isinstance(value.get("data"), dict):
        raise RuntimeError("product success has no object data")
    return value


def node_facts(value, files):
    """核对实际扫描计数，拒绝未知值；大整数十进制字符串沿用 wire 兼容。"""
    node = envelope(value)["data"]["node"]
    if int(node["files"]) != files or int(node["directories"]) != 2:
        raise RuntimeError("product root does not match the actual isolated filesystem")
    if node.get("subtree_bytes") is None:
        raise RuntimeError("actual product root size is unobserved")
    return node


def run(checkout, output, environment, cli, mcp):
    """同一绝对期限完成真实扫描、跨进程查询与后台任务，不直接访问 store。"""
    if os.getuid() == 0:
        raise RuntimeError("product flows must run as the original ordinary UID")
    output.mkdir(parents=True, exist_ok=False)
    root = output / "scope-目录"
    (root / "sub").mkdir(parents=True)
    (root / "alpha").write_bytes(b"abc")
    (root / "sub/beta").write_bytes(b"abcde")
    (root / "leaf-文件").write_bytes(b"abcdefg")
    data = output / "data"
    deadline = time.monotonic() + 90
    identities = {"cli": hashlib.sha256(cli.read_bytes()).hexdigest(),
                  "mcp": hashlib.sha256(mcp.read_bytes()).hexdigest()}

    def remaining():
        left = deadline - time.monotonic()
        if left <= 0:
            raise RuntimeError("original product qualification deadline exhausted")
        return left

    def command(name, arguments):
        with (output / (name + ".stdout")).open("wb") as stdout, (output / (name + ".stderr")).open("wb") as stderr:
            result = subprocess.run([str(cli), "--json", *arguments], cwd=checkout, env=environment,
                                    stdout=stdout, stderr=stderr, timeout=remaining())
        if result.returncode != 0:
            raise RuntimeError(name + " exited nonzero; inspect original logs")
        rows = (output / (name + ".stdout")).read_text().splitlines()
        if len(rows) != 1:
            raise RuntimeError(name + " did not emit exactly one product envelope")
        return envelope(json.loads(rows[0]))

    initial = command("cli-init", ["init", "--root", str(root), "--data-dir", str(data), "--index-only"])
    index = initial["data"]["index"]
    if index["state"] != "completed" or int(index["files"]) != 3:
        raise RuntimeError("CLI init did not complete the actual three-file scan")
    scope, revision = index["scope_id"], index["revision_id"]
    before = command("cli-node-before", ["--data-dir", str(data), "node", "--scope", scope])
    expected = node_facts(before, 3)
    if before["data"]["revision_id"] != revision:
        raise RuntimeError("CLI init/query revision identity differs")
    buffer = bytearray()
    with (output / "mcp.stderr").open("wb") as stderr, (output / "mcp.stdout").open("wb") as transcript:
        process = subprocess.Popen([str(mcp), "--data-dir", str(data), "--transport", "stdio", "--profile", "manage"],
                                   cwd=checkout, env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=stderr, bufsize=0)
        request_id = 0
        try:
            def rpc(method, params):
                nonlocal request_id
                request_id += 1
                request = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
                encoded = json.dumps(request).encode() + b"\n"
                if process.stdin.write(encoded) != len(encoded):
                    raise RuntimeError("MCP fixture request was not fully written")
                process.stdin.flush()
                while b"\n" not in buffer:
                    if not select.select([process.stdout], [], [], remaining())[0]:
                        raise RuntimeError("MCP response deadline exhausted")
                    chunk = os.read(process.stdout.fileno(), 65536)
                    if not chunk:
                        raise RuntimeError("MCP exited before the requested response")
                    transcript.write(chunk)
                    transcript.flush()
                    buffer.extend(chunk)
                    if len(buffer) > 1 << 20:
                        raise RuntimeError("MCP product response exceeds the fixed fixture budget")
                line, _, tail = buffer.partition(b"\n")
                buffer[:] = tail
                value = json.loads(line)
                if value.get("id") != request_id or "error" in value or "result" not in value:
                    raise RuntimeError("MCP returned a mismatched or failed protocol response")
                return value["result"]

            def tool(name, arguments):
                result = rpc("tools/call", {"name": name, "arguments": arguments})
                if result.get("isError") is not False:
                    raise RuntimeError("MCP product tool failed")
                return envelope(result["structuredContent"])

            rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                               "clientInfo": {"name": "native-product-qualification", "version": "1"}})
            observed = tool("diskgraph_node", {"scope": scope})
            if observed.get("revision_id") != revision or observed.get("server_id") != before.get("server_id"):
                raise RuntimeError("CLI/MCP published revision or server identity differs")
            if node_facts(observed, 3) != expected:
                raise RuntimeError("CLI/MCP disagree on the same published root")
            (root / "new-file").write_bytes(b"added")
            job = tool("diskgraph_index", {"scope": scope})["data"]["job_id"]
            while True:
                status = tool("diskgraph_status", {"job_id": job})["data"]
                if status["state"] == "completed":
                    break
                if status["state"] in {"failed", "cancelled"}:
                    raise RuntimeError("actual MCP background scan failed")
                time.sleep(min(0.02, remaining()))
            after = tool("diskgraph_node", {"scope": scope})
            node_facts(after, 4)
            if not after.get("revision_id", "").startswith("rev-" + job + "-"):
                raise RuntimeError("new MCP revision is not bound to the completed actual job")
            process.stdin.close()
            if process.wait(timeout=remaining()) != 0:
                raise RuntimeError("MCP did not exit normally after EOF and completed job")
        finally:
            if process.poll() is None:
                # 仅失败夹具的监督处置；不将强制退出计为产品正常恢复或通过。
                process.kill()
                process.wait(timeout=10)
    final = command("cli-node-after", ["--data-dir", str(data), "node", "--scope", scope])
    if final["data"]["revision_id"] != after["revision_id"] or node_facts(final, 4) != after["data"]["node"]:
        raise RuntimeError("CLI did not observe the actual MCP-published revision")
    for name, binary in (("cli", cli), ("mcp", mcp)):
        if hashlib.sha256(binary.read_bytes()).hexdigest() != identities[name]:
            raise RuntimeError("product binary identity changed during qualification")
    result = {"production_acceptance": False, "status": "passed", "binary_sha256": identities,
              "scope_id": scope, "initial_revision": revision, "job_id": job,
              "final_revision": after["revision_id"], "uid": os.getuid(),
              "cli_normal_exits": 3, "mcp_normal_exit": True, "actual_files_before": 3, "actual_files_after": 4}
    (output / "receipt.json").write_text(json.dumps(result, indent=2) + "\n")
    return result
