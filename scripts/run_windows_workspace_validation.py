"""Windows 台式机 workspace 验收；每段保留独立 CI 期限。"""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time

from windows_acceptance_job import run_owned

PHASE_SECONDS = 1800
PHASES = (
    ('foundation', ('diskgraph-core', 'diskgraph-store', 'diskgraph-disktree',
                    'diskgraph-scan-worker', 'diskgraph-ffi', 'diskgraph-testkit', 'diskgraph-ops')),
    ('entry', ('diskgraph-engine', 'diskgraph-cli', 'diskgraph-mcp')),
)

def phase_command(phase):
    """返回指定验收阶段的 Cargo 参数。"""
    packages = dict(PHASES)[phase]
    return ['cargo', 'test', *[argument for package in packages for argument in ('-p', package)],
            '--all-targets', '--locked', '--no-fail-fast', '--', '--nocapture']

def write_receipt(output, receipt):
    """原子更新本轮回执；未执行或回收不确定的阶段不得标记通过。"""
    temporary = output / 'receipt.tmp'
    temporary.write_text(json.dumps(receipt, indent=2), encoding='utf-8')
    os.replace(temporary, output / 'receipt.json')

def run_phases(output, owner, source):
    """每段独立入 Job；失败仍执行后段，只有回收不确定才停止。"""
    output.mkdir(exist_ok=False)
    receipt = {'source': source, 'phases': [], 'passed': False,
               'production_qualification': False, 'phase_seconds': PHASE_SECONDS}
    write_receipt(output, receipt)
    for phase, _ in PHASES:
        record = {'phase': phase, 'command': phase_command(phase), 'exit_code': None,
                  'timed_out': False, 'job_retirement_confirmed': False}
        receipt['phases'].append(record)
        write_receipt(output, receipt)
        start = time.monotonic()
        with (output / (phase + '.log')).open('xb') as log:
            try:
                result = owner([sys.executable, str(pathlib.Path(__file__).resolve()), '--phase', phase],
                               timeout=PHASE_SECONDS, stdout=log, stderr=subprocess.STDOUT)
                record['exit_code'] = result.returncode
                record['job_retirement_confirmed'] = True
            except subprocess.TimeoutExpired as error:
                record['timed_out'] = True
                # 原 owner 只在 Job 退休及管道回收失败时附加 note；保守拒绝任何不确定状态。
                record['cleanup_notes'] = list(getattr(error, '__notes__', []))
                record['job_retirement_confirmed'] = not record['cleanup_notes']
            except Exception as error:
                record['error_type'] = type(error).__name__
                record['cleanup_notes'] = list(getattr(error, '__notes__', []))
            finally:
                record['elapsed_seconds'] = time.monotonic() - start
                write_receipt(output, receipt)
        print(json.dumps(record), flush=True)
        if not record['job_retirement_confirmed']:
            break
    receipt['passed'] = len(receipt['phases']) == len(PHASES) and all(
        r['exit_code'] == 0 and not r['timed_out'] and r['job_retirement_confirmed']
        for r in receipt['phases'])
    write_receipt(output, receipt)
    return receipt

def main():
    """启动本轮 Windows 原生测试；worker/driver/C 夹具由现有准入脚本先准备。"""
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--output-dir', type=pathlib.Path)
    group.add_argument('--phase', choices=[phase for phase, _ in PHASES])
    args = parser.parse_args()
    if sys.platform != 'win32':
        parser.error('native Windows validation requires Windows')
    root = pathlib.Path(__file__).resolve().parents[1]
    os.chdir(root)
    if args.phase:
        return subprocess.call(phase_command(args.phase))
    # 使用已经通过准入的真实镜像；不能把缺失夹具的跳过当成完整验收。
    for name in ('DISKGRAPH_SCAN_WORKER_PATH', 'DISKGRAPH_SCAN_DRIVER_FIXTURE',
                 'DISKGRAPH_WINDOWS_EXIT_C0000000_FIXTURE', 'DISKGRAPH_WINDOWS_EXIT_C0000005_FIXTURE'):
        if not pathlib.Path(os.environ.get(name, '')).is_file():
            parser.error('missing prepared native fixture: ' + name)
    diff = subprocess.check_output(['git', 'diff', '--binary', 'HEAD', '--', 'crates', 'scripts', '.github', 'Cargo.toml', 'Cargo.lock'])
    source = {'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
              'tracked_diff_sha256': hashlib.sha256(diff).hexdigest(),
              'harness_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
              'source_status': subprocess.check_output(['git', 'status', '--porcelain', '--', 'crates', 'scripts', '.github', 'Cargo.toml', 'Cargo.lock'], text=True)}
    return 0 if run_phases(args.output_dir.resolve(), run_owned, source)['passed'] else 1

if __name__ == '__main__':
    sys.exit(main())
