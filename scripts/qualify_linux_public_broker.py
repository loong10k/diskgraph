"""仅CI隔离root域验证真实broker/CLI/worker；可运行临时unit，不安装桌面服务，不把失败kill当退休证据。"""
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
import tarfile
import platform
import io
import uuid
import contextlib
import sys
import time


def lowered():
    os.setgroups([])
    os.setgid(23112)
    os.setuid(23112)


def lowered_service():
    os.setgroups([])
    os.setgid(23111)
    os.setuid(23111)


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


def direct_query(root, data, command):
    """仅隔离业务对照；由实际专用UID执行固定CLI并消耗同一Popen wait。"""
    child = subprocess.Popen([str(root / "diskgraph"), "--json", "--data-dir", str(data), *command],
                             preexec_fn=lowered_service, env={}, cwd=root, stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        stdout, stderr = child.communicate(timeout=35)
        assert len(stdout) + len(stderr) <= 1 << 20
        return child.returncode, json.loads(stdout)
    finally:
        original_dispose(child)


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
            while not Path(config["socket_path"]).exists() or (Path(config["socket_path"]).stat().st_mode & 0o777) != 0o666:
                assert time.monotonic() - started < 30, "original startup never bound socket"
                assert server.poll() is None, server.communicate()
                time.sleep(0.005)
            command = {
                "scope_list": ["scope", "list"],
                "snapshots": ["snapshots"],
                "unknown_job": ["status", "--job", "absent-original-job"],
            }.get(mode, ["doctor"])
            baseline = None
            if mode in ("scope_show", "scope_show_revoked"):
                fixture_root = root / "private_scope"
                fixture_root.mkdir(mode=0o700)
                os.chown(fixture_root, 23111, 23111)
                registered = direct_query(root, data, ["scope", "add", "--root", str(fixture_root)])
                assert registered[0] == 0 and registered[1]["ok"], registered
                scope_id = registered[1]["data"]["scope_id"]
                if mode == "scope_show_revoked":
                    removed = direct_query(root, data, ["scope", "remove", "--scope", scope_id])
                    assert removed[0] == 0 and removed[1]["ok"], removed
                command = ["scope", "show", "--scope", scope_id]
            if mode in ("scope_list", "snapshots", "unknown_job", "scope_show", "scope_show_revoked"):
                baseline = direct_query(root, data, command)
                if mode == "scope_show":
                    assert baseline[0] == 0 and baseline[1]["data"]["root_display"] == str(fixture_root), baseline
                elif mode == "scope_show_revoked":
                    assert baseline[0] == 3 and baseline[1]["error"]["code"] == "permission_denied", baseline
            frontend = subprocess.Popen([str(root / "diskgraph"), "--json", "--supervisor-socket", config["socket_path"], *command],
                                        preexec_fn=lowered, env={}, cwd=root, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            before = time.monotonic()
            stdout, stderr = frontend.communicate(timeout=35)
            assert time.monotonic() - before < 33 and len(stdout) + len(stderr) <= 1 << 20
            document = json.loads(stdout)
            if baseline is not None:
                assert frontend.returncode == baseline[0] and document["ok"] == baseline[1]["ok"], (baseline, stdout, stderr)
                if document["ok"]:
                    assert document["data"] == baseline[1]["data"], (baseline, document)
                else:
                    assert document["error"]["code"] == baseline[1]["error"]["code"], (baseline, document)
                assert (data / "diskgraph-control.sqlite").exists()
            elif mode == "clean":
                assert frontend.returncode == 0 and document["ok"] is True, (frontend.returncode, stdout, stderr)
                assert (data / "diskgraph-control.sqlite").exists()
            else:
                assert frontend.returncode != 0 and document["ok"] is False
                assert document["error"]["code"] == "recovery_unconfirmed", document
                assert not (data / "diskgraph-control.sqlite").exists()
            # 仅原broker实际正常wait后断言CLEAN，不用测试kill或client状态替代监督退休。
            server_stdout, server_stderr = server.communicate(timeout=35)
            assert server.returncode == 0, (server_stdout, server_stderr)
            assert (state / "bootstrap.slot").read_bytes() == (b"DGSL01C\n" if mode != "active" else original_record)
            assert (state / "uid_23111.slot").read_bytes() == b"DGSL01C\n"
            return {"mode": mode, "frontend_exit": frontend.returncode, "server_exit": server.returncode,
                    "frontend_stdout_eof": True, "original_broker_wait_consumed": True,
                    "bootstrap_record": (state / "bootstrap.slot").read_text().strip(), "json_ok": document["ok"],
                    "direct_business_result_matched": baseline is not None}
        finally:
            if frontend is not None:
                original_dispose(frontend)
            original_dispose(server)


def wire_query(args, mode):
    """实际原时钟doctor正控及单字段负控；负例不得创建Engine或认领原槽。"""
    with tempfile.TemporaryDirectory(prefix="dg-wire-", dir="/var/lib") as folder:
        root = Path(folder)
        os.chmod(root, 0o755)
        state, data, config = prepare(root, args)
        helper = root / "wire_client.py"
        shutil.copyfile(Path(__file__).with_name("qualify_linux_broker_wire_client.py"), helper)
        os.chmod(helper, 0o555)
        server = subprocess.Popen([str(root / "broker"), "--config", str(root / "config.json"), "--once"],
                                  env={}, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        frontend = None
        try:
            started = time.monotonic()
            while not Path(config["socket_path"]).exists() or (Path(config["socket_path"]).stat().st_mode & 0o777) != 0o666:
                assert time.monotonic() - started < 30, "original wire startup never bound"
                assert server.poll() is None, server.communicate()
                time.sleep(0.005)
            frontend = subprocess.Popen([sys.executable, str(helper),
                                         config["socket_path"], mode],
                                        preexec_fn=lowered, env={}, cwd=root, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            stdout, stderr = frontend.communicate(timeout=35)
            assert frontend.returncode == 0, (stdout, stderr)
            row = json.loads(stdout)
            server_stdout, server_stderr = server.communicate(timeout=35)
            assert server.returncode == 0, (server_stdout, server_stderr)
            assert (data / "diskgraph-control.sqlite").exists() == (mode == "doctor")
            for name in ("bootstrap.slot", "uid_23111.slot"):
                assert (state / name).read_bytes() == b"DGSL01C\n"
            return {**row, "original_server_wait_consumed": True,
                    "database_not_born": mode != "doctor", "original_slots_clean": True}
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




def systemd_installation(prefix, config, retained):
    # 仅临时Linux runner独占unit；不enable，不操作其他服务，失败清理不能充当原资源退休。
    from prepare_linux_supervisor_installation import write_at, verify_root_directory
    unit_name = 'dg-isolated-' + uuid.uuid4().hex + '.service'
    directory = os.open('/run/systemd/system', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    slots, frontend = [], None
    installed = False
    try:
        verify_root_directory(directory)
        body = (prefix / 'diskgraph-supervisor.service').read_bytes()
        write_at(directory, unit_name, io.BytesIO(body), 4096, 0o644)
        installed = True
        os.fsync(directory)
        for name in ['bootstrap.slot', 'uid_23111.slot']:
            slots.append(os.open(prefix / 'state' / name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC))
        subprocess.run(['systemctl', 'daemon-reload'], check=True, timeout=20, capture_output=True)
        subprocess.run(['systemctl', 'start', unit_name], check=True, timeout=20, capture_output=True)
        endpoint = Path(config['socket_path'])
        started = time.monotonic()
        while not endpoint.exists() or (endpoint.stat().st_mode & 0o777) != 0o666:
            assert time.monotonic() - started < 30, 'actual service never bound original socket'
            time.sleep(0.005)
        frontend = subprocess.Popen([str(prefix / 'images/diskgraph'), '--json', '--supervisor-socket', str(endpoint), 'doctor'],
                                    preexec_fn=lowered, env={}, cwd=prefix, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        stdout, stderr = frontend.communicate(timeout=35)
        assert frontend.returncode == 0 and json.loads(stdout)['ok'] is True, (stdout, stderr)
        retirement = time.monotonic() + 5
        while any(os.pread(fd, 8, 0) != b'DGSL01C\n' for fd in slots):
            assert time.monotonic() < retirement, 'original service owner did not retire both original slots'
            time.sleep(0.005)
        subprocess.run(['systemctl', 'stop', unit_name], check=True, timeout=35, capture_output=True)
        assert all(os.pread(fd, 8, 0) == b'DGSL01C\n' for fd in slots)
        return {'generated_unit_started': True, 'actual_frontend_exit': 0,
                'original_slot_fds_clean_before_stop': True, 'desktop_service_installed': False,
                'boot_enable_performed': False}
    finally:
        try:
            if frontend is not None:
                original_dispose(frontend)
            if installed:
                # 仅退出夹具：只有上面的实际正控能够记录退休通过。
                try:
                    subprocess.run(['systemctl', 'stop', unit_name], check=True, timeout=35, capture_output=True)
                except BaseException:
                    # 原服务尚未确认停止时保留unit、安装树与槽位，不能通过删夹具丢失恢复责任。
                    retained['retain'] = True
                    raise
                else:
                    os.unlink(unit_name, dir_fd=directory)
                    os.fsync(directory)
                    subprocess.run(['systemctl', 'daemon-reload'], check=True, timeout=20, capture_output=True)
        finally:
            for fd in slots:
                os.close(fd)
            os.close(directory)


@contextlib.contextmanager
def installed_workspace():
    retained = {'root': Path(tempfile.mkdtemp(prefix='dg-installed-', dir='/var/lib')), 'retain': False}
    try:
        yield retained
    finally:
        if retained['retain']:
            print('unretired isolated service installation retained at ' + str(retained['root']), file=sys.stderr)
        else:
            shutil.rmtree(retained['root'])


def prepared_installation(args):
    # 实际四镜像归档由独立安装者摘要准入；不复制手写配置代替prepare路径。
    from prepare_linux_supervisor_installation import prepare, PIN
    target = {'x86_64': 'x86_64-unknown-linux-gnu', 'aarch64': 'aarch64-unknown-linux-gnu'}[platform.machine()]
    with installed_workspace() as retained:
        root = retained['root']
        os.chmod(root, 0o755)
        archive_path = root / 'product.tar.gz'
        images = {'diskgraph': args.cli, 'diskgraph-mcp': args.mcp,
                  'diskgraph-scan-worker': args.worker, 'diskgraph-supervisor': args.broker}
        worker = {'schema_version': 1, 'target': target, 'protocol_version': 2,
                  'pinned_scanner_revision': PIN, 'executable': {'name': 'diskgraph-scan-worker',
                  'bytes': args.worker.stat().st_size, 'sha256': digest(args.worker)}}
        with tarfile.open(archive_path, 'w:gz') as archive:
            for name, source in images.items():
                archive.add(source, arcname='diskgraph-' + target + '/bin/' + name, recursive=False)
            body = json.dumps(worker).encode()
            info = tarfile.TarInfo('diskgraph-' + target + '/bin/scan-worker-manifest.json')
            info.size = len(body)
            archive.addfile(info, io.BytesIO(body))
        expected = digest(archive_path)
        prefix = root / 'installed'
        receipt = prepare(archive_path, expected, prefix, 23111, 23111, [23112], with_broker=True)
        assert receipt['broker_artifacts_prepared'] and not receipt['services_started']
        config_path = prefix / 'supervisor-config.json'
        assert config_path.stat().st_uid == 0 and (config_path.stat().st_mode & 0o7777) == 0o600
        # 原生解析实际unit，但不将静态verify冒充systemd运行或安装验收。
        subprocess.run(['systemd-analyze', 'verify', str(prefix / 'diskgraph-supervisor.service')],
                       check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        config = json.loads(config_path.read_text())
        server = subprocess.Popen([str(prefix / 'images/diskgraph-supervisor'), '--config', str(config_path), '--once'],
                                  env={}, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        frontend = None
        try:
            started = time.monotonic()
            endpoint = Path(config['socket_path'])
            while not endpoint.exists() or (endpoint.stat().st_mode & 0o777) != 0o666:
                assert time.monotonic() - started < 30 and server.poll() is None
                time.sleep(0.005)
            frontend = subprocess.Popen([str(prefix / 'images/diskgraph'), '--json', '--supervisor-socket', str(endpoint), 'doctor'],
                                        preexec_fn=lowered, env={}, cwd=root, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            stdout, stderr = frontend.communicate(timeout=35)
            assert frontend.returncode == 0 and json.loads(stdout)['ok'] is True, (stdout, stderr)
            server_stdout, server_stderr = server.communicate(timeout=35)
            assert server.returncode == 0, (server_stdout, server_stderr)
            assert (prefix / 'state/bootstrap.slot').read_bytes() == b'DGSL01C\n'
            assert (prefix / 'state/uid_23111.slot').read_bytes() == b'DGSL01C\n'
            original_config = config_path.read_bytes()
            try:
                prepare(archive_path, expected, prefix, 23111, 23111, [23112], with_broker=True)
            except FileExistsError:
                pass
            else:
                raise AssertionError('installer replaced existing original deployment')
            assert config_path.read_bytes() == original_config
            result = {'mode': 'actual_prepared_installation', 'frontend_exit': 0, 'original_broker_wait_consumed': True,
                      'static_unit_verified': True, 'systemd_service_started': False, 'repeat_install_refused': True}
            if args.systemd:
                result['systemd_runtime'] = systemd_installation(prefix, config, retained)
                result['systemd_service_started'] = True
            return result
        finally:
            if frontend is not None:
                original_dispose(frontend)
            original_dispose(server)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--broker", type=Path, required=True)
    parser.add_argument("--mcp", type=Path, help="actual fourth installed image for preparation qualification")
    parser.add_argument("--systemd", action="store_true", help="run generated unit only inside explicit isolated Linux root acceptance")
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    if args.systemd and args.mcp is None:
        parser.error("--systemd requires the actual installed MCP image via --mcp")
    if os.geteuid() != 0 or os.environ.get("DG_ISOLATED_ROOT_ACCEPTANCE") != "1":
        raise SystemExit("requires explicit isolated root acceptance; never installs desktop service")
    rows = []
    def observed(row):
        rows.append(row)
        # 逐案留证：后续安装失败也保留之前实际执行的结果，不把整体失败改写成通过。
        print(json.dumps({"qualified_case": row}, sort_keys=True), flush=True)
    for mode in ("clean", "active", "bad_digest", "scope_list", "snapshots", "unknown_job", "scope_show", "scope_show_revoked"):
        observed(actual_broker(args, mode))
    for lower in (False, True):
        observed(delegated_sender(args, lower))
    for mode in ("doctor", "override", "unknown_command", "duplicate", "bad_numeric", "write", "unknown_field"):
        observed(wire_query(args, mode))
    if args.mcp is not None:
        observed(prepared_installation(args))
    receipt = {"passed": len(rows), "failed": 0, "actual_product_binaries": True, "system_service_installed": args.systemd,
               "desktop_service_installed": False, "isolated_runner_only": True,
               "active_pending_owner_injected": False, "platform": "Linux", "cases": rows,
               "binary_sha256": {name: digest(getattr(args, name)) for name in ("cli", "worker", "broker")}}
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
