"""仅CI隔离root域验证真实broker/CLI/worker；不安装系统服务，不把失败kill当退休证据。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import time


def lowered():
    os.setgroups([])
    os.setgid(23112)
    os.setuid(23112)


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def original_dispose(child):
    # 仅失败夹具处置同一Popen对象，不能据此声称原监督资源完成。
    if child.poll() is None:
        child.kill()
        child.wait(timeout=10)


def prepare(root, args, active=False, bad_digest=False):
    for name, source in [("diskgraph", args.cli), ("worker", args.worker), ("broker", args.broker)]:
        shutil.copyfile(source, root / name)
        os.chmod(root / name, 0o555)
    state = root / "state"
    state.mkdir(mode=0o755)
    (state / "bootstrap.slot").write_bytes(b"DGSL01A\n" if active else b"DGSL01C\n")
    os.chmod(state / "bootstrap.slot", 0o600)
    (state / "uid_23111.slot").write_bytes(b"DGSL01C\n")
    os.chmod(state / "uid_23111.slot", 0o600)
    os.chown(state / "uid_23111.slot", 23111, 23111)
    data = state / "data"
    data.mkdir(mode=0o700)
    os.chown(data, 23111, 23111)
    config = {"schema_version": 1, "frontend_uid": 23112, "service_uid": 23111, "service_gid": 23111,
              "state_dir": str(state), "data_dir": str(data), "socket_path": str(root / "front.sock"),
              "cli_path": str(root / "diskgraph"), "cli_sha256": "0" * 64 if bad_digest else digest(root / "diskgraph"),
              "worker_path": str(root / "worker"), "worker_sha256": digest(root / "worker")}
    (root / "config.json").write_text(json.dumps(config), encoding="utf-8")
    os.chmod(root / "config.json", 0o600)
    return state, data, config


def actual_broker(args, mode):
    with tempfile.TemporaryDirectory(prefix="dg-broker-", dir="/var/lib") as folder:
        root = Path(folder)
        os.chmod(root, 0o755)
        state, data, config = prepare(root, args, mode == "active", mode == "bad_digest")
        original_record = (state / "bootstrap.slot").read_bytes()
        server = subprocess.Popen([str(root / "broker"), "--config", str(root / "config.json"), "--once"],
                                  env={}, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        frontend = None
        try:
            started = time.monotonic()
            if mode == "bad_digest":
                stdout, stderr = server.communicate(timeout=35)
                assert server.returncode == 10 and b"SHA256 mismatch" in stderr, (stdout, stderr)
                assert not Path(config["socket_path"]).exists() and not (data / "diskgraph-control.sqlite").exists()
                assert (state / "bootstrap.slot").read_bytes() == original_record
                return {"mode": mode, "server_exit": server.returncode, "socket_not_born": True, "database_not_born": True}
            while not Path(config["socket_path"]).exists():
                assert time.monotonic() - started < 30, "original startup never bound socket"
                assert server.poll() is None, server.communicate()
                time.sleep(0.005)
            frontend = subprocess.Popen([str(root / "diskgraph"), "--json", "--supervisor-socket", config["socket_path"], "doctor"],
                                        preexec_fn=lowered, env={}, cwd=root, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            before = time.monotonic()
            stdout, stderr = frontend.communicate(timeout=35)
            assert time.monotonic() - before < 33 and len(stdout) + len(stderr) <= 1 << 20
            document = json.loads(stdout)
            if mode == "clean":
                assert frontend.returncode == 0 and document["ok"] is True, (frontend.returncode, stdout, stderr)
                assert (data / "diskgraph-control.sqlite").exists()
            else:
                assert frontend.returncode != 0 and document["ok"] is False
                assert document["error"]["code"] == "recovery_unconfirmed", document
                assert not (data / "diskgraph-control.sqlite").exists()
            # 仅原broker实际正常wait后断言CLEAN，不用测试kill或client状态替代监督退休。
            server_stdout, server_stderr = server.communicate(timeout=35)
            assert server.returncode == 0, (server_stdout, server_stderr)
            assert (state / "bootstrap.slot").read_bytes() == (b"DGSL01C\n" if mode == "clean" else original_record)
            assert (state / "uid_23111.slot").read_bytes() == b"DGSL01C\n"
            return {"mode": mode, "frontend_exit": frontend.returncode, "server_exit": server.returncode,
                    "frontend_stdout_eof": True, "original_broker_wait_consumed": True,
                    "bootstrap_record": (state / "bootstrap.slot").read_text().strip(), "json_ok": document["ok"]}
        finally:
            if frontend is not None:
                original_dispose(frontend)
            original_dispose(server)


def delegated_sender(args, lower):
    # root创建的真实listener被移交：SO_PEERCRED仍呈现创建者，实际发送PID/UID必须另行拒绝。
    with tempfile.TemporaryDirectory(prefix="dg-peer-", dir="/var/lib") as folder:
        root = Path(folder)
        os.chmod(root, 0o755)
        cli = root / "diskgraph"
        shutil.copyfile(args.cli, cli)
        os.chmod(cli, 0o555)
        endpoint = root / "front.sock"
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(str(endpoint))
        listener.listen(1)
        os.chmod(endpoint, 0o666)
        code = """import socket,sys,struct
s=socket.socket(fileno=int(sys.argv[1]));c,_=s.accept()
while c.recv(4096): pass
try: c.sendall(bytes([3])+struct.pack('!I',4)+struct.pack('!i',0))
except (BrokenPipeError,ConnectionResetError): pass
c.close();s.close()
"""
        sender = subprocess.Popen(["/usr/bin/python3", "-c", code, str(listener.fileno())], pass_fds=(listener.fileno(),),
                                  preexec_fn=lowered if lower else None, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        frontend = subprocess.Popen([str(cli), "--json", "--supervisor-socket", str(endpoint), "doctor"],
                                    preexec_fn=lowered, env={}, cwd=root, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            stdout, stderr = frontend.communicate(timeout=35)
            assert frontend.returncode == 10 and not stdout, (frontend.returncode, stdout, stderr)
            sender_stdout, sender_stderr = sender.communicate(timeout=5)
            assert sender.returncode == 0, (sender_stdout, sender_stderr)
            return {"mode": "delegated_nonroot_sender" if lower else "delegated_same_root_uid_sender",
                    "root_listener_original_pid": os.getpid(), "actual_sender_original_pid": sender.pid,
                    "frontend_exit": frontend.returncode, "stdout_bytes": len(stdout), "original_sender_wait_consumed": True}
        finally:
            original_dispose(frontend)
            original_dispose(sender)
            listener.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--broker", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if os.geteuid() != 0 or os.environ.get("DG_ISOLATED_ROOT_ACCEPTANCE") != "1":
        raise SystemExit("requires explicit isolated root acceptance; never installs desktop service")
    rows = [actual_broker(args, mode) for mode in ("clean", "active", "bad_digest")]
    rows += [delegated_sender(args, lower) for lower in (False, True)]
    receipt = {"passed": len(rows), "failed": 0, "actual_product_binaries": True, "system_service_installed": False,
               "active_pending_owner_injected": False, "platform": "Linux", "cases": rows,
               "binary_sha256": {name: digest(getattr(args, name)) for name in ("cli", "worker", "broker")}}
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
