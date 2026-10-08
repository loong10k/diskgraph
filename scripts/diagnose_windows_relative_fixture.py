#!/usr/bin/env python3
"""Windows 相对目录句柄 A/B/B/A 夹具诊断，不修改正式负载验收。"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def measure(mode, files):
    """保留文件集合及实际临时目录退出，完整验证独立计时。"""
    load = load_module('readonly_load', ROOT / 'scripts/accept-readonly-load.py')
    native = load_module('relative_fixture', ROOT / 'scripts/windows_relative_fixture.py')
    whole_started = time.perf_counter()
    with tempfile.TemporaryDirectory(prefix='diskgraph-relative-diagnostic-') as temporary:
        root = Path(temporary)
        create_started = time.perf_counter()
        if mode == 'path':
            load.create_fixture(root, files, workers=1)
        else:
            with native.WindowsRelativeFixture(root) as fixture:
                fixture.create(files)
        create_seconds = time.perf_counter() - create_started
        verify_started = time.perf_counter()
        expected = {native.fixture_name(i) for i in range(files)}
        if {p.name for p in root.iterdir()} != expected:
            raise RuntimeError('fixture name/count mismatch')
        for name in expected:
            path = root / name
            if path.stat().st_nlink != 1 or path.read_bytes() != b'x' * 32:
                raise RuntimeError('fixture contents/link count mismatch')
        verify_seconds = time.perf_counter() - verify_started
        cleanup_started = time.perf_counter()
        if mode == 'path':
            load.retire_fixture(root, files, workers=1)
        else:
            with native.WindowsRelativeFixture(root) as fixture:
                fixture.remove(files)
        if list(root.iterdir()):
            raise RuntimeError('fixture deletion did not complete')
    return {'mode': mode, 'files': files, 'bytes_per_file': 32,
            'create_seconds': create_seconds, 'verify_seconds': verify_seconds,
            'cleanup_seconds': time.perf_counter() - cleanup_started,
            'whole_seconds': time.perf_counter() - whole_started,
            'temporary_directory_exit_completed': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--files', type=int, default=2500)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if os.name != 'nt' or not 1 <= args.files <= 200000:
        parser.error('native Windows and 1..200000 actual files required')
    paths = ['scripts/accept-readonly-load.py', 'scripts/windows_relative_fixture.py',
             'scripts/diagnose_windows_relative_fixture.py', 'scripts/tests/test_windows_relative_fixture.py']
    receipt = {'status': 'running', 'purpose': 'diagnostic only; not production acceptance',
               'platform': platform.platform(), 'python': platform.python_version(),
               'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
               'source_sha256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in paths},
               'cases': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        args.output.write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    save()
    try:
        for mode in ('path', 'relative', 'relative', 'path'):
            receipt['cases'].append(measure(mode, args.files))
            save()
            print(json.dumps(receipt['cases'][-1]), flush=True)
        receipt['status'] = 'complete'
    except BaseException as error:
        receipt['status'] = 'failed'
        try:
            save()
        except OSError as report_error:
            error.add_note(f'diagnostic receipt failed: {report_error}')
        raise
    save()
    print(json.dumps(receipt), flush=True)


if __name__ == '__main__':
    main()
