"""隔离root域验证真实单连接guardian和客户CLI；不安装服务，不把失败处置记为退休。"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import platform
import time
import signal

from qualify_linux_public_broker import original_dispose


def install_candidate(workspace, args):
    """以当前四镜像和完整归档摘要进入真实独占安装器；不手写配置替代安装。"""
    from prepare_linux_supervisor_installation import prepare, PIN
    target = {'aarch64': 'aarch64-unknown-linux-gnu', 'x86_64': 'x86_64-unknown-linux-gnu'}[platform.machine()]
    archive = workspace / 'candidate.tar.gz'
    images = {'diskgraph': args.cli, 'diskgraph-mcp': args.mcp,
              'diskgraph-scan-worker': args.worker, 'diskgraph-supervisor': args.broker}
    worker_bytes = args.worker.read_bytes()
    manifest = {'schema_version': 1, 'target': target, 'protocol_version': 2,
                'pinned_scanner_revision': PIN, 'executable': {'name': 'diskgraph-scan-worker',
                'bytes': len(worker_bytes), 'sha256': hashlib.sha256(worker_bytes).hexdigest()}}
    with tarfile.open(archive, 'w:gz') as package:
        for name, path in images.items():
            package.add(path, arcname='diskgraph-' + target + '/bin/' + name, recursive=False)
        raw = json.dumps(manifest).encode()
        member = tarfile.TarInfo('diskgraph-' + target + '/bin/scan-worker-manifest.json')
        member.size = len(raw)
        package.addfile(member, io.BytesIO(raw))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    prefix = workspace / 'installation'
    receipt = prepare(archive, digest, prefix, 23111, 23111, [23112],
                      with_guardian=True, frontend_gid=23112)
    if receipt['services_started'] or receipt['data_owner'] != [23112, 23112]:
        raise AssertionError('guardian installation must remain unenabled with actual customer data')
    for name in ('diskgraph', 'diskgraph-mcp', 'diskgraph-scan-worker'):
        if receipt['role_images'][name]['mode'] != 0o511:
            raise AssertionError('customer-readable installed executor image')
    return prefix


def customer():
    """固定夹具身份；真实辅助组也必须进入内核连接凭据。"""
    os.setgroups([23114, 23115])
    os.setgid(23112)
    os.setuid(23112)


def qualify(args, disconnect=False, stop_before_query=False):
    """返回真实CLI、原槽和原guardian正常wait结果；异常不删除未确认安装树。"""
    if sys.platform != 'linux' or os.geteuid() != 0 or os.environ.get('DG_ISOLATED_ROOT_ACCEPTANCE') != '1':
        raise RuntimeError('isolated Linux root qualification only')
    workspace = Path(tempfile.mkdtemp(prefix='dg-guardian-', dir='/var/lib'))
    workspace.chmod(0o755)
    success = False
    guardian = frontend = None
    result = {'scope': 'accepted Unix connection -> customer doctor CLI -> original wait',
              'production_ready': False, 'mcp_qualified': False, 'systemd_qualified': False,
              'scan_qualified': False, 'services_installed': False}
    try:
        root = install_candidate(workspace, args)
        state, data = root / 'state', root / 'state/data'
        config_path = root / 'supervisor-config.json'
        config = json.loads(config_path.read_text())
        helper = root / 'wire_client.py'
        shutil.copyfile(Path(__file__).with_name('qualify_linux_broker_wire_client.py'), helper)
        helper.chmod(0o555)
        bootstrap = os.open(state / 'bootstrap.slot', os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            original = os.fstat(bootstrap)
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                # 必须先于客户connect/send启用；不能事后补凭据或转交其他客户连接。
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_PASSCRED, 1)
                listener.bind(config['socket_path'])
                os.chmod(config['socket_path'], 0o666)
                listener.listen(1)
                listener.settimeout(30)
                command = [sys.executable, str(helper), config['socket_path'],
                           'stop_before_query' if stop_before_query else 'disconnect'] if (
                           stop_before_query or disconnect) else [
                           str(root / 'images/diskgraph'), '--json', '--supervisor-socket',
                           config['socket_path'], 'doctor']
                frontend = subprocess.Popen(command,
                                            preexec_fn=customer, env={}, cwd=root,
                                            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                stream, _ = listener.accept()
                with stream:
                    guardian = subprocess.Popen([str(root / 'images/diskgraph-supervisor'), '--config', str(config_path),
                                                 '--connection'], stdin=stream, env={}, cwd=root,
                                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                if stop_before_query:
                    until = time.monotonic() + 30
                    while True:
                        if guardian.poll() is not None or time.monotonic() >= until:
                            raise AssertionError('original guardian did not install stop handler')
                        status = Path('/proc/' + str(guardian.pid) + '/status').read_text()
                        caught = int(next(line.split(':', 1)[1].strip() for line in status.splitlines()
                                          if line.startswith('SigCgt:')), 16)
                        if caught & (1 << (signal.SIGTERM - 1)):
                            break
                        time.sleep(0.001)
                    guardian.send_signal(signal.SIGTERM)
                output, errors = frontend.communicate(timeout=35)
                if frontend.returncode != 0:
                    raise AssertionError(('customer query failed', frontend.returncode, output, errors))
                row = json.loads(output)
                if not (stop_before_query or disconnect) and row.get('ok') is not True:
                    raise AssertionError('actual CLI frontend did not return successful doctor envelope')
                guardian_output, guardian_errors = guardian.communicate(timeout=35)
                expected_exits = (10,) if stop_before_query else (0, 10) if disconnect else (0,)
                if guardian.returncode not in expected_exits:
                    raise AssertionError(('original guardian failed', guardian_output, guardian_errors))
                if os.pread(bootstrap, 8, 0) != b'DGSL01C\n':
                    raise AssertionError('original bootstrap not CLEAN after original guardian wait')
                current = os.fstat(bootstrap)
                if (original.st_dev, original.st_ino) != (current.st_dev, current.st_ino):
                    raise AssertionError('original bootstrap changed')
                databases = list(data.glob('*.sqlite*'))
                if stop_before_query and databases:
                    raise AssertionError('stopped admission must not create customer databases')
                if not stop_before_query and (not databases or any(path.stat().st_uid != 23112 for path in databases)):
                    raise AssertionError('databases must be created by real customer executor')
                if (state / 'uid_23111.slot').read_bytes() != b'DGSL01C\n':
                    raise AssertionError('customer executor touched legacy service capacity')
                result.update(passed=True, customer=row, customer_database_uid=23112,
                              original_guardian_waited=True, original_bootstrap_clean=True,
                              legacy_service_slot_untouched=True, public_disconnected=disconnect,
                              stopped_before_query=stop_before_query, actual_installer_used=True)
                success = True
                return result
        finally:
            os.close(bootstrap)
    finally:
        # 超时只处置夹具，保留安装树/ACTIVE供诊断；从不将kill/wait当产品恢复证据。
        for child in (frontend, guardian):
            if child is not None:
                original_dispose(child)
        if success:
            shutil.rmtree(workspace)
        else:
            print('unconfirmed fixture retained: ' + str(workspace), file=sys.stderr)


def main():
    parser = argparse.ArgumentParser()
    for option in ('cli', 'worker', 'broker', 'mcp', 'receipt'):
        parser.add_argument('--' + option, type=Path, required=True)
    parser.add_argument('--systemd', action='store_true', help='qualify actual generated socket/template only on isolated runner')
    args = parser.parse_args()
    result = {'cases': [qualify(args), qualify(args, disconnect=True),
                        qualify(args, stop_before_query=True)], 'production_ready': False}
    if args.systemd:
        from qualify_linux_guardian_unit import qualify_unit
        result['systemd_runtime'] = qualify_unit(args)
    args.receipt.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
