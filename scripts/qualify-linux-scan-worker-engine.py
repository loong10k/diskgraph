#!/usr/bin/env python3
"""在原生外层PID namespace内验证当前Engine，退出后由原init owner实际回收。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import traceback

from assemble_scan_worker_host import assemble


def run_step(output, root, environment, receipt, name, command, capture=False):
    """记录固定阶段原结果；关闭日志的次错不能覆盖原命令失败。"""
    log = (output / (name + '.log')).open('wb')
    try:
        result = subprocess.run(command, cwd=root, env=environment, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE if capture else log, stderr=log, check=True)
    finally:
        primary = sys.exception()
        try:
            log.close()
        except BaseException as error:
            if primary is None:
                raise
            receipt.setdefault('secondary_errors', []).append(
                {'stage': name + '-log-close', 'kind': type(error).__name__, 'errno': getattr(error, 'errno', None)})
    if capture:
        (output / (name + '.jsonl')).write_bytes(result.stdout)
        return result.stdout
    return (output / (name + '.log')).read_bytes()


def main(output):
    output = output.resolve()
    outer = os.environ.get('DG_NATIVE_NAMESPACE_OUTPUT')
    if (sys.platform != 'linux' or os.getuid() == 0 or os.geteuid() == 0
            or os.getpid() != 1 or os.readlink('/proc/self') != '1'
            or os.environ.get('DG_NATIVE_NAMESPACE_SUPERVISED') != '1'
            or os.readlink('/proc/self/ns/pid') != os.environ.get('DG_NATIVE_PID_NAMESPACE')
            or not outer or output != Path(outer).resolve() / 'runner/qualification'):
        raise ValueError('actual ordinary-user supervised namespace init is required')
    output.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[1]
    receipt = {'schema_version': 1, 'status': 'incomplete', 'executed_parent_cases': 0,
               'security_acceptance': False, 'actual_pid': 1,
               'pid_namespace': os.readlink('/proc/self/ns/pid'),
               'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()}
    environment = dict(os.environ)
    environment['CARGO_TARGET_DIR'] = str(output / 'target')

    try:
        # 只在已降权的隔离checkout装配；根路径、上游、每份原/新源码均有指纹约束。
        assemble(root, output)
        manifest = root / 'crates/diskgraph-engine/integration_candidates/scan_worker_host/current.json'
        receipt['source_manifest_sha256'] = hashlib.sha256(manifest.read_bytes()).hexdigest()
        receipt['qualifier_sha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
        rows = run_step(output, root, environment, receipt, 'build', ['cargo', 'build', '--offline', '--locked', '-p', 'diskgraph-scan-worker',
                             '--bin', 'diskgraph-scan-worker', '--message-format=json'], True)
        artifacts = [json.loads(line) for line in rows.splitlines()]
        executables = [r['executable'] for r in artifacts if r.get('reason') == 'compiler-artifact'
                       and r.get('target', {}).get('name') == 'diskgraph-scan-worker' and r.get('executable')]
        if len(executables) != 1:
            raise ValueError('exact one actual Cargo worker artifact is required')
        worker = Path(executables[0])
        if not worker.is_absolute() or worker.is_symlink() or not worker.is_file():
            raise ValueError('actual regular absolute worker artifact is required')
        with worker.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        receipt['worker_sha256'] = digest
        receipt['worker_bytes'] = worker.stat().st_size
        shutil.copyfile(worker, output / 'actual-scan-worker')
        environment['DISKGRAPH_ENGINE_SCAN_WORKER'] = str(worker)
        cases = [('engine-flow', ['--test', 'scan_worker_engine_flow'], 3),
                 ('recovery', ['--lib', 'scan_worker_recovery_tests'], 2)]
        for name, target, count in cases:
            raw = run_step(output, root, environment, receipt, name, ['cargo', 'test', '--offline', '--locked', '-p', 'diskgraph-engine', *target,
                             '--', '--nocapture', '--test-threads=1'])
            expected = f'test result: ok. {count} passed; 0 failed; 0 ignored;'.encode()
            if expected not in raw:
                raise ValueError(name + ': actual exact native count did not pass')
            receipt[name] = {'passed': count, 'failed': 0, 'ignored': 0,
                             'raw_log_sha256': hashlib.sha256(raw).hexdigest()}
        receipt.update(status='component_tests_passed_awaiting_outer_cleanup', executed_parent_cases=5)
    except BaseException as error:
        receipt.update(status='failed', primary_error={'kind': type(error).__name__, 'repr': repr(error)},
                       traceback=traceback.format_exc())
        raise
    finally:
        primary = sys.exception()
        try:
            (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
        except BaseException as error:
            if primary is None:
                raise
            print('receipt secondary error: ' + repr(error), file=sys.stderr)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output-dir', type=Path, required=True)
    main(parser.parse_args().output_dir)
