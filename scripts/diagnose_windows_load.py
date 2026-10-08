#!/usr/bin/env python3
"""Windows完整200k负载阶段诊断；正式300秒门禁保持失败，不提供生产验收。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from windows_acceptance_job import run_owned

ROOT = Path(__file__).resolve().parents[1]


def run(bin_dir, output):
    """执行同源码release完整负载并保留阶段输出；原失败不被回执失败覆盖。"""
    bin_dir = bin_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    images = {}
    for name in ('diskgraph.exe', 'diskgraph-mcp.exe', 'diskgraph-scan-worker.exe'):
        with (bin_dir / name).open('rb') as stream:
            images[name] = hashlib.file_digest(stream, 'sha256').hexdigest()
    command = [sys.executable, str(ROOT / 'scripts/accept-readonly-load.py'),
               '--bin-dir', str(bin_dir), '--files', '200000']
    started = time.monotonic()
    receipt = {'command': command, 'started_unix_seconds': time.time(),
               'clean_environment_confirmed': False,
               'preceding_formal_acceptance': 'failed; retirement uncertainty may affect diagnostic',
               'inner_cli_timeout_seconds': 300,
               'status': 'running', 'production_acceptance': False,
               'purpose': 'diagnostic only; formal acceptance remains independent',
               'commit': os.environ.get('GITHUB_SHA'), 'binary_sha256': images,
               'formal_timeout_seconds': 300, 'diagnostic_timeout_seconds': 900,
               'source_sha256': {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                                 for path in [Path(__file__).resolve(), ROOT / 'scripts/accept-readonly-load.py',
                                              ROOT / 'scripts/windows_acceptance_job.py']}}
    def save():
        receipt['elapsed_seconds'] = time.monotonic() - started
        receipt['observed_unix_seconds'] = time.time()
        (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    def capture(stdout, stderr):
        for name, data in [('stdout.log', stdout), ('stderr.log', stderr)]:
            if data is not None:
                text = data.decode('utf-8', errors='replace') if isinstance(data, bytes) else data
                (output / name).write_text(text, encoding='utf-8')
    save()
    try:
        result = run_owned(command,
                           cwd=ROOT, capture_output=True, text=True, timeout=900)
        capture(result.stdout, result.stderr)
        if result.returncode:
            raise RuntimeError('full-load diagnostic command failed; see captured phase logs')
        report = json.loads(result.stdout)
        if not (report.get('files') == 200000 and report.get('indexed_nodes') == 200001
                and report.get('queries') == 32 and report.get('concurrent_clients') == 4
                and report.get('passed') == report.get('total') == 6
                and len(report.get('checks', {})) == 6 and all(report['checks'].values())):
            raise RuntimeError('full-load diagnostic lacks complete workload coverage')
        receipt['observed_load'] = report
        receipt['status'] = 'complete'
        save()
        return receipt
    except BaseException as error:
        receipt['status'] = 'failed'
        try:
            if isinstance(error, subprocess.TimeoutExpired):
                capture(error.stdout, error.stderr)
            save()
        except OSError as secondary:
            error.add_note(f'diagnostic receipt unavailable: {secondary}')
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'win32':
        parser.error('native Windows required; host mocks do not qualify Windows')
    run(args.bin_dir, args.output_dir)


if __name__ == '__main__':
    main()
