#!/usr/bin/env python3
"""隔离root验收原子监督出生C层；不安装服务、不创建Engine、不替代Rust原owner或broker。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile


def demote():
    os.setgroups([])
    os.setgid(23111)
    os.setuid(23111)


def copy_fixture(source, destination):
    """独占新夹具镜像，以固定块约束实际字节，返回可核对的摘要；不执行来源任意命令。"""
    digest, total = hashlib.sha256(), 0
    with source.open('rb') as src, destination.open('xb') as dst:
        while True:
            chunk = src.read(65536)
            if not chunk:
                break
            total += len(chunk)
            if total > 64 << 20:
                raise ValueError('fixture exceeds image budget')
            dst.write(chunk)
            digest.update(chunk)
    destination.chmod(0o555)
    return {'bytes': total, 'sha256': digest.hexdigest()}


def run_driver(arguments, root, drop_identity=False):
    """保留实际driver直到原pidfd wait完成；观察超时不杀死其唯一child owner。"""
    process = subprocess.Popen(arguments, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, cwd=root, preexec_fn=demote if drop_identity else None)
    try:
        stdout, stderr = process.communicate(timeout=10)
        return process.returncode, stdout, stderr
    finally:
        if process.poll() is None:
            process.wait()


def qualify(driver, role, output, baseline=None):
    """运行固定场景并记录原pidfd实际wait；失败保留原结果，不把startup Ready当成功exec。"""
    if sys.platform != 'linux' or any(v != 0 for v in (os.getuid(), os.geteuid(), os.getgid(), os.getegid())):
        raise RuntimeError('requires isolated Linux root')
    for entry in ('/', '/var', '/var/lib'):
        info = os.lstat(entry)
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022:
            raise RuntimeError('fixture ancestry is not root protected: ' + entry)
    root = Path(tempfile.mkdtemp(prefix='diskgraph-root-spawn-', dir='/var/lib'))
    root.chmod(0o755)
    result = {'scope': 'native64 C atomic supervisor birth only', 'rust_owner_qualified': False,
              'product_broker_qualified': False, 'bootstrap_lifetime_qualified': False,
              'actual_cli_mcp_qualified': False, 'services_installed': False, 'cases': []}
    try:
        result['images'] = {name: copy_fixture(source, root / name) for name, source in (('driver', driver), ('role', role))}
        if baseline is not None:
            result['images']['baseline'] = copy_fixture(baseline, root / 'baseline')
            exit_code, stdout, stderr = run_driver([str(root / 'baseline'), str(root / 'role'), 'clean'], root)
            records = [json.loads(line) for line in stdout.splitlines()]
            red = exit_code == 42 and records[0]['passed'] is False and records[0]['uid'] == 0 and records[0]['private_endpoints'] is False and records[-1]['original_pidfd_waited']
            result['baseline'] = {'contract_missing_reproduced': red, 'exit_code': exit_code,
                                  'stdout': stdout, 'stderr': stderr, 'records': records}
            if not red:
                raise AssertionError('baseline did not reproduce the missing contract')
        malformed = root / 'malformed_role'
        malformed.write_bytes(b'not an ELF image\n')
        malformed.chmod(0o555)
        worker = root / 'worker'
        worker.write_bytes(b'DG_HELD_WORKER\n')
        worker.chmod(0o511)
        private_role = root / 'private_role'
        result['images']['private_role'] = copy_fixture(role, private_role)
        private_role.chmod(0o511)
        errors = {'invalid_protection': 22, 'root_uid': 1, 'root_gid': 1, 'sentinel_uid': 1, 'sentinel_gid': 1,
                  'aliased_fd': 22, 'low_fd': 22, 'nonroot': 1,
                  'missing_groups': 22, 'oversized_groups': 22, 'sentinel_group': 22,
                  'low_worker': 22, 'aliased_worker': 22, 'writable_worker': 22}
        for case in ('clean', 'supplementary', 'held_worker', 'executor_private', 'executor_public', 'executor_acl', 'denied_access', *errors, 'bad_image', 'denied_groups', 'signal_after_ready'):
            arguments = [str(root / 'driver'), str(malformed if case == 'bad_image' else private_role if case == 'executor_private' else root / 'role'), case]
            if case == 'held_worker': arguments.append(str(worker))
            exit_code, stdout, stderr = run_driver(arguments, root, case == 'nonroot')
            records = [json.loads(line) for line in stdout.splitlines()]
            acl_verified = case != 'executor_acl' or records[:1] == [{'acl_read_permission_verified': True}]
            proof_records = records[1:] if case == 'executor_acl' else records
            if case in ('clean', 'supplementary'):
                passed = exit_code == 0 and records[0]['passed'] and records[-1]['original_pidfd_waited'] and records[-1]['public_eof_before_init'] and records[-1]['exit_code'] == 0
                passed = passed and records[0]['groups'] == (2 if case == 'supplementary' else 0)
            elif case == 'executor_private':
                passed = exit_code == 0 and records[0] == {'passed': True, 'dumpable': 0} and records[-1]['original_pidfd_waited'] and records[-1]['handshake'] and records[-1]['exit_code'] == 0
            elif case in ('executor_public', 'executor_acl', 'denied_access'):
                passed = acl_verified and exit_code == 0 and len(proof_records) == 2 and proof_records[0]['startup_kind'] == 2 and proof_records[0]['startup_phase'] == 19 and proof_records[0]['startup_errno'] == (38 if case == 'denied_access' else 13) and proof_records[-1]['original_pidfd_waited'] and not records[-1]['handshake'] and records[-1]['exit_code'] == 127
            elif case == 'held_worker':
                passed = exit_code == 0 and records[0] == {'held_worker_read': True, 'worker_path_eacces': True, 'passed': True} and records[-1]['original_pidfd_waited'] and records[-1]['public_eof_before_init']
            elif case == 'signal_after_ready':
                passed = exit_code == 0 and len(records) == 3 and records[0] == {'original_pidfd_signal_sent': True, 'signal_errno': 0} and records[-1]['original_pidfd_waited'] and records[-1]['public_eof_before_init'] and not records[-1]['handshake'] and not records[-1]['exited_normally'] and records[-1]['exit_code'] == 9
            elif case == 'denied_groups':
                passed = exit_code == 42 and len(records) == 2 and records[0]['startup_after_ack_bytes'] == 12 and records[0]['startup_kind'] == 2 and records[0]['startup_phase'] == 15 and records[0]['startup_errno'] == 1 and records[-1]['original_pidfd_waited'] and records[-1]['public_eof_before_init'] and not records[-1]['handshake'] and records[-1]['exited_normally'] and records[-1]['exit_code'] == 127
            elif case == 'bad_image':
                passed = exit_code == 42 and records[-1]['original_pidfd_waited'] and records[-1]['exit_code'] == 127 and records[-2]['startup_phase'] == 11 and records[-2]['startup_errno'] == 8
            else:
                passed = exit_code == 0 and records == [{'before_birth_rejected': True,
                          'errno': errors[case], 'pidfd_unchanged': True}]
            result['cases'].append({'case': case, 'passed': bool(passed), 'exit_code': exit_code,
                                    'stdout': stdout, 'stderr': stderr, 'records': records})
        result['passed'] = all(case['passed'] for case in result['cases'])
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(result, indent=2) + '\n')
        if not result['passed']:
            raise AssertionError('native supervisor spawn fixture failed; see receipt')
    finally:
        shutil.rmtree(root)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--driver', required=True, type=Path)
    parser.add_argument('--role', required=True, type=Path)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    qualify(args.driver.resolve(), args.role.resolve(), args.output.resolve(),
            args.baseline.resolve() if args.baseline is not None else None)
