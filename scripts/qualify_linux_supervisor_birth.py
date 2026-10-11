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
import stat
import struct
from functools import partial
import subprocess
import sys
import tempfile
import time

SERVICE_UID = 23111
FRONTEND_UID = 23112


def demote(primary_gid=SERVICE_UID, groups=()):
    os.setgroups(list(groups))
    os.setgid(primary_gid)
    os.setuid(SERVICE_UID)


def run_case(prefix, binary, worker, case):
    """只运行固定 doctor 镜像与命令；保留原 child 到实际 communicate/wait 完成。"""
    root = prefix / case
    root.mkdir(mode=0o755); root.chmod(0o755)
    state = root / 'state'; state.mkdir(mode=0o755); state.chmod(0o755)
    slot = state / ('uid_' + str(SERVICE_UID) + '.slot')
    slot.write_bytes(b'DGSL01A\n' if case == 'active_slot' else b'DGSL01C\n')
    slot.chmod(0o600); os.chown(slot, SERVICE_UID, SERVICE_UID)
    data = root / 'data'; data.mkdir(mode=0o700); os.chown(data, SERVICE_UID, SERVICE_UID)
    data = data / 'database'
    parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_DGRAM)
    control_parent, control_child = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    control_parent.setsockopt(socket.SOL_SOCKET, socket.SO_PASSCRED, 1)
    # 内核必须在交付之前准备接收凭据；实际接收对象由原子继承，不由命名入口替代。
    child.setsockopt(socket.SOL_SOCKET, socket.SO_PASSCRED, 1)
    files = [os.open(state, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW),
             os.open('/proc/self/ns/user', os.O_RDONLY), os.open('/proc/self/ns/mnt', os.O_RDONLY)]
    domain_file = os.open('/proc/self/ns/time', os.O_RDONLY)
    domain = os.fstat(domain_file); os.close(domain_file)
    request = {'schema_version': 2, 'control_fd': control_child.fileno(), 'nonce': os.urandom(32).hex(), 'service_uid': SERVICE_UID, 'service_gid': SERVICE_UID,
               'frontend_uid': FRONTEND_UID, 'state_root': str(state), 'data_dir': str(data),
               'worker_path': str(worker), 'worker_sha256': hashlib.sha256(worker.read_bytes()).hexdigest(),
               'worker_bytes': worker.stat().st_size,
               'deadline': {'version': 1, 'domain': [1, domain.st_dev, domain.st_ino],
                            'expires_nanos': time.monotonic_ns() + 30_000_000_000 - 1}}
    if case == 'wrong_type_control': request['control_fd'] = files[0]
    if case == 'aliased_control': request['control_fd'] = child.fileno()
    if case == 'stdio_control': request['control_fd'] = 1
    if case == 'legacy_control': request['schema_version'] = 1
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
                                   pass_fds=(child.fileno(), control_child.fileno(), *((files[0],) if case == 'wrong_type_control' else ())), preexec_fn=partial(demote, primary_gid=0 if case == 'wrong_gid' else SERVICE_UID,
                                                     groups=(0,) if case == 'supplementary_groups' else ()))
        child.close(); control_child.close()
        if case not in ('forged_role', 'expired', 'stdio_control', 'legacy_control', 'wrong_gid', 'supplementary_groups'):
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
        # 不能用 SO_PEERCRED 创建者身份或 JSON 自报身份认证实际写入者。
        # 仅接受原 child 的逐消息内核 PID/UID/GID；拒绝截断、附加 FD 与超额字节。
        controls = bytearray()
        actual_credentials = []
        while True:
            chunk, ancillary, flags, _ = control_parent.recvmsg(4096, 256)
            if flags & (socket.MSG_TRUNC | socket.MSG_CTRUNC): raise AssertionError('control truncated')
            if not chunk: break
            credentials = [struct.unpack('3i', value) for level, kind, value in ancillary
                           if level == socket.SOL_SOCKET and kind == socket.SCM_CREDENTIALS]
            if len(ancillary) != 1 or credentials != [(process.pid, SERVICE_UID, SERVICE_UID)]:
                raise AssertionError('control writer does not match original child')
            actual_credentials.extend(credentials)
            controls.extend(chunk)
            if len(controls) > 65536: raise AssertionError('control byte budget exceeded')
        frames = []
        while controls:
            if len(controls) < 4: raise AssertionError('partial control prefix')
            size = struct.unpack('>I', controls[:4])[0]
            if not 0 < size <= 4096 or len(controls) < 4 + size: raise AssertionError('invalid control body')
            frames.append(json.loads(controls[4:4+size])); del controls[:4+size]
            if len(frames) > 64: raise AssertionError('control frame budget exceeded')
        for sequence, frame in enumerate(frames):
            if frame['version'] != 1 or frame['session'] != list(hashlib.sha256(raw).digest()) or frame['sequence'] != sequence:
                raise AssertionError('control version, session or sequence mismatch')
        replies = [json.loads(line) for line in out.decode().splitlines()]
        positive = case == 'clean'
        code = replies[0].get('error', {}).get('code') if len(replies) == 1 else None
        # 业务数据来自现有 doctor；拒绝须先于 SQLite 出生，不能用任意错误替代身份/容量语义。
        exact = {'active_slot': 'recovery_unconfirmed', 'forged_role': 'permission_denied',
                 'wrong_session': 'permission_denied', 'wrong_sender': 'permission_denied', 'wrong_data': 'permission_denied',
                 'expired': 'budget_exceeded', 'wrong_image': 'conflict',
                 'aliased_control': 'permission_denied', 'stdio_control': 'permission_denied',
                 'legacy_control': 'permission_denied', 'wrong_type_control': 'permission_denied',
                 'wrong_gid': 'permission_denied', 'supplementary_groups': 'permission_denied'}
        passed = (process.returncode == 0 and len(replies) == 1 and replies[0].get('ok') is True
                  and (data / 'diskgraph.sqlite').is_file() and slot.read_bytes() == b'DGSL01C\n') if positive else (
                  process.returncode != 0 and not actual_data.exists()
                  and (case not in exact or code == exact[case]))
        if positive:
            passed = passed and [f['notification']['kind'] for f in frames] == ['ready', 'foreground_ended', 'cleanup_complete']
            passed = passed and frames[1]['notification'] == {'kind': 'foreground_ended', 'exit_code': 0, 'panicked': False}
        else:
            passed = passed and not frames
        if case == 'active_slot': passed = passed and slot.read_bytes() == b'DGSL01A\n'
        return {'case': case, 'passed': passed, 'exit_code': process.returncode,
                'error_code': code, 'elapsed_seconds': time.monotonic() - started,
                'data_born': actual_data.exists(), 'slot_record': slot.read_bytes().decode(),
                'stdout': out.decode(), 'stderr': err.decode(), 'original_child_waited': True,
                'private_frames': frames, 'actual_control_credentials': actual_credentials}
    finally:
        child.close(); parent.close(); control_child.close(); control_parent.close()
        for descriptor in files: os.close(descriptor)
        # timeout 不杀死原进程再宣称完成；等待原 doctor owner 自行退出，失败保留完整原错误。
        if process is not None and process.poll() is None: process.wait()


def qualify(binary, worker, output):
    """接受当前构建的两个明确镜像，只在通过预检的全新受保护夹具内执行；不注册服务。"""
    if sys.platform != 'linux' or os.geteuid() != 0 or os.getuid() != 0 or os.getgid() != 0:
        raise RuntimeError('qualification requires isolated Linux root')
    # runner 的 /opt 可能是可写工具缓存；不能用它伪装为生产要求的 root 受保护域。
    ancestry = {}
    for entry in ('/', '/var', '/var/lib', '/opt'):
        info = os.lstat(entry)
        ancestry[entry] = {'uid': info.st_uid, 'mode': stat.S_IMODE(info.st_mode),
                           'is_directory': stat.S_ISDIR(info.st_mode)}
    for entry in ('/', '/var', '/var/lib'):
        info = ancestry[entry]
        if info['uid'] != 0 or info['mode'] & 0o022 or not info['is_directory']:
            raise RuntimeError('fixture ancestry is not independently root protected: ' + entry)
    prefix = Path(tempfile.mkdtemp(prefix='diskgraph-birth-', dir='/var/lib'))
    prefix.chmod(0o755)
    images = prefix / 'images'; images.mkdir(mode=0o755); images.chmod(0o755)
    receipts = {}
    for name, source in [('diskgraph', binary), ('diskgraph-scan-worker', worker)]:
        image = images / name
        with source.open('rb') as src, image.open('xb') as dst: shutil.copyfileobj(src, dst)
        image.chmod(0o555)
        receipts[name] = {'bytes': image.stat().st_size, 'sha256': hashlib.sha256(image.read_bytes()).hexdigest()}
    result = {'schema_version': 1, 'scope': 'authenticated Linux doctor birth and original retirement notification only',
              'public_broker_qualified': False, 'pending_frontend_exit_qualified': False,
              'services_registered': False, 'fixture_ancestry': ancestry,
              'images': receipts, 'cases': []}
    try:
        for case in ('clean', 'active_slot', 'forged_role', 'wrong_session', 'wrong_sender', 'expired', 'wrong_image', 'wrong_data', 'aliased_control', 'stdio_control', 'legacy_control', 'wrong_type_control', 'wrong_gid', 'supplementary_groups'):
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
