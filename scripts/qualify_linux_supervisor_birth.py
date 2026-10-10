#!/usr/bin/env python3
"""隔离 root/专用 UID 夹具验证真实 CLI 出生接线；不是公共 broker 或服务安装验收。"""
import argparse
import array
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time

SERVICE_UID = 23111
FRONTEND_UID = 23112


def demote():
    os.setgroups([])
    os.setgid(SERVICE_UID)
    os.setuid(SERVICE_UID)


def run_case(prefix, binary, worker, case):
    """只运行固定 doctor 镜像与命令；保留原 child 到实际 communicate/wait 完成。"""
    root = prefix / case
    root.mkdir(mode=0o755)
    state = root / 'state'; state.mkdir(mode=0o755)
    slot = state / ('uid_' + str(SERVICE_UID) + '.slot')
    slot.write_bytes(b'DGSL01A\n' if case == 'active_slot' else b'DGSL01C\n')
    slot.chmod(0o600); os.chown(slot, SERVICE_UID, SERVICE_UID)
    data = root / 'data'; data.mkdir(mode=0o700); os.chown(data, SERVICE_UID, SERVICE_UID)
    data = data / 'database'
    parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_DGRAM)
    # 内核必须在交付之前准备接收凭据；实际接收对象由原子继承，不由命名入口替代。
    child.setsockopt(socket.SOL_SOCKET, socket.SO_PASSCRED, 1)
    files = [os.open(state, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW),
             os.open('/proc/self/ns/user', os.O_RDONLY), os.open('/proc/self/ns/mnt', os.O_RDONLY)]
    domain_file = os.open('/proc/self/ns/time', os.O_RDONLY)
    domain = os.fstat(domain_file); os.close(domain_file)
    request = {'schema_version': 1, 'nonce': os.urandom(32).hex(), 'service_uid': SERVICE_UID,
               'frontend_uid': FRONTEND_UID, 'state_root': str(state), 'data_dir': str(data),
               'worker_path': str(worker), 'worker_sha256': hashlib.sha256(worker.read_bytes()).hexdigest(),
               'worker_bytes': worker.stat().st_size,
               'deadline': {'version': 1, 'domain': [1, domain.st_dev, domain.st_ino],
                            'expires_nanos': time.monotonic_ns() + 30_000_000_000 - 1}}
    if case == 'expired': request['deadline']['expires_nanos'] = time.monotonic_ns() - 1
    if case == 'wrong_image': request['worker_sha256'] = '00' * 32
    raw = json.dumps(request, separators=(',', ':')).encode()
    session = hashlib.sha256(raw).digest()
    if case == 'wrong_session': session = bytes(32)
    if case == 'forged_role': raw = b'{}'
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(root),
           'DISKGRAPH_LINUX_SUPERVISOR_BIRTH': raw.decode(),
           'DISKGRAPH_LINUX_SUPERVISOR_FD': str(child.fileno())}
    actual_data = data if case != 'wrong_data' else data.parent / 'other_database'
    started = time.monotonic()
    process = None
    try:
        process = subprocess.Popen([str(binary), '--json', '--data-dir', str(actual_data), 'doctor'],
                                   env=env, cwd=root, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   pass_fds=(child.fileno(),), preexec_fn=demote)
        child.close()
        if case not in ('forged_role', 'expired'):
            if case == 'wrong_sender':
                sender_pid = os.fork()
                if sender_pid == 0:
                    try:
                        os.setgroups([]); os.setgid(FRONTEND_UID); os.setuid(FRONTEND_UID)
                        parent.sendmsg([b'DGSM01A\n' + session], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', files))])
                    finally:
                        os._exit(0)
                _, sender_status = os.waitpid(sender_pid, 0)
                assert sender_status == 0
            else:
                parent.sendmsg([b'DGSM01A\n' + session],
                               [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array('i', files))])
        out, err = process.communicate(timeout=40)
        replies = [json.loads(line) for line in out.decode().splitlines()]
        positive = case == 'clean'
        code = replies[0].get('error', {}).get('code') if len(replies) == 1 else None
        # 业务数据来自现有 doctor；拒绝须先于 SQLite 出生，不能用任意错误替代身份/容量语义。
        exact = {'active_slot': 'recovery_unconfirmed', 'forged_role': 'permission_denied',
                 'wrong_session': 'permission_denied', 'wrong_sender': 'permission_denied', 'wrong_data': 'permission_denied',
                 'expired': 'budget_exceeded'}
        passed = (process.returncode == 0 and len(replies) == 1 and replies[0].get('ok') is True
                  and (data / 'diskgraph.sqlite').is_file() and slot.read_bytes() == b'DGSL01C\n') if positive else (
                  process.returncode != 0 and not actual_data.exists()
                  and (case not in exact or code == exact[case]))
        if case == 'active_slot': passed = passed and slot.read_bytes() == b'DGSL01A\n'
        return {'case': case, 'passed': passed, 'exit_code': process.returncode,
                'error_code': code, 'elapsed_seconds': time.monotonic() - started,
                'data_born': actual_data.exists(), 'slot_record': slot.read_bytes().decode(),
                'stdout': out.decode(), 'stderr': err.decode(), 'original_child_waited': True}
    finally:
        child.close(); parent.close()
        for descriptor in files: os.close(descriptor)
        # timeout 不杀死原进程再宣称完成；等待原 doctor owner 自行退出，失败保留完整原错误。
        if process is not None and process.poll() is None: process.wait()


def qualify(binary, worker, output):
    """接受当前构建的两个明确镜像，只在全新 /opt 夹具内复制执行；不注册任何服务。"""
    if sys.platform != 'linux' or os.geteuid() != 0 or os.getuid() != 0 or os.getgid() != 0:
        raise RuntimeError('qualification requires isolated Linux root')
    prefix = Path(tempfile.mkdtemp(prefix='diskgraph-birth-', dir='/opt'))
    prefix.chmod(0o755)
    images = prefix / 'images'; images.mkdir(mode=0o755)
    receipts = {}
    for name, source in [('diskgraph', binary), ('diskgraph-scan-worker', worker)]:
        image = images / name
        with source.open('rb') as src, image.open('xb') as dst: shutil.copyfileobj(src, dst)
        image.chmod(0o555)
        receipts[name] = {'bytes': image.stat().st_size, 'sha256': hashlib.sha256(image.read_bytes()).hexdigest()}
    result = {'schema_version': 1, 'scope': 'authenticated Linux doctor constructor only',
              'public_broker_qualified': False, 'pending_frontend_exit_qualified': False,
              'services_registered': False, 'images': receipts, 'cases': []}
    try:
        for case in ('clean', 'active_slot', 'forged_role', 'wrong_session', 'wrong_sender', 'expired', 'wrong_image', 'wrong_data'):
            result['cases'].append(run_case(prefix, images / 'diskgraph', images / 'diskgraph-scan-worker', case))
        result['passed'] = all(case['passed'] for case in result['cases'])
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(result, indent=2) + '\n')
        if not result['passed']: raise AssertionError('native constructor qualification failed; see receipt')
    finally:
        # 每个 child 已实际 wait；这里只清理本轮独占创建的隔离夹具，不触及已有安装。
        shutil.rmtree(prefix)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    qualify(args.binary.resolve(), args.worker.resolve(), args.output.resolve())
