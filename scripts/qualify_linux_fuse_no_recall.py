#!/usr/bin/env python3
"""在私有 ARM64 Docker 容器中验收当前提交的 FUSE 数据访问拒绝。"""
import argparse
import hashlib
import io
import json
import platform
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
IMAGE = 'rust@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97'
FIXTURE = ROOT / 'docs/benchmarks/linux_content_no_recall_audit_2026_10_08'


def verify_output(text, product, provider):
    """参数为三个真实日志；返回通过，缺少场景或真实回收则拒绝。"""
    names = ['actual_provider_content_read_must_not_fetch',
             'actual_provider_digest_must_not_fetch',
             'nested_provider_content_read_must_not_fetch',
             'nested_provider_digest_must_not_fetch']
    if 'test result: ok. 4 passed; 0 failed; 0 ignored;' not in product:
        raise RuntimeError('four actual provider tests must pass without ignoring')
    if any(f'test {name} ...' not in product for name in names):
        raise RuntimeError('direct and nested read/digest cases must execute')
    if product.count('before=0 after=0 safe=true') != 4:
        raise RuntimeError('each product case must refuse without recall')
    if provider.count('FUSE_DATA_OPEN') != 1 or provider.count('FUSE_FETCH_DATA=1') != 1:
        raise RuntimeError('ordinary positive control must open and fetch exactly once')
    if 'PRODUCT_TEST_EXIT 0' not in text or 'ORIGINAL_PROVIDER_UNMOUNTED_AND_REAPED=1' not in text:
        raise RuntimeError('actual product exit and original provider cleanup required')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', required=True, type=Path)
    args = parser.parse_args()
    if platform.machine().lower() not in ('arm64', 'aarch64'):
        raise RuntimeError('pinned fixture image requires native ARM64; no emulated acceptance')
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', commit], cwd=ROOT)
    # 只验收确切已提交源码，不把工作区改动静默混入提交身份。
    # 异常或外部超时不得删除仍可能被原容器使用的源码；只在真实回收通过后清理。
    source = Path(tempfile.mkdtemp(prefix='diskgraph-fuse-source-'))
    with tarfile.open(fileobj=io.BytesIO(archive)) as package:
        package.extractall(source, filter='data')
    fixture = source / FIXTURE.relative_to(ROOT)
    for name in ('fuse_provider.c', 'run_product_probe.py'):
        shutil.copyfile(fixture / name, output / name)
    shutil.copyfile(fixture / 'engine_no_recall_probe.rs',
                    source / 'crates/diskgraph-engine/tests/linux_content_no_recall_probe.rs')
    command = ['docker', 'run', '--rm', '--cidfile', str(output / 'container.cid'),
               '--cap-add=SYS_ADMIN', '--security-opt', 'apparmor=unconfined',
               '--device-cgroup-rule=c 10:229 rwm',
               '-v', f'{source}:/src:ro', '-v', f'{output}:/fixture',
               '-w', '/src', '-e', 'CARGO_TARGET_DIR=/tmp/target',
               '-e', 'DG_FUSE_TEST_TIMEOUT=1200', IMAGE,
               '/bin/bash', '-c',
               'gcc -Wall -Wextra -Werror /fixture/fuse_provider.c -o /tmp/fuse-provider && python3 /fixture/run_product_probe.py']
    pending = {'commit': commit, 'image': IMAGE, 'retained_source': str(source),
               'status': 'launched or unconfirmed; no cleanup qualification',
               'container_identity_file': str(output / 'container.cid'),
               'fixture_sha256': {name: hashlib.sha256((fixture / name).read_bytes()).hexdigest()
                                  for name in ('fuse_provider.c', 'engine_no_recall_probe.rs', 'run_product_probe.py')}}
    (output / 'receipt.json').write_text(json.dumps(pending, indent=2) + '\n')
    with (output / 'orchestration.log').open('wb') as log:
        try:
            result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=1500)
        except subprocess.TimeoutExpired:
            pending['status'] = 'client observation timeout; original container completion unconfirmed; source retained'
            (output / 'receipt.json').write_text(json.dumps(pending, indent=2) + '\n')
            raise
    receipt = {'commit': commit, 'image': IMAGE, 'exit': result.returncode, 'verified': False,
               'qualification': 'limited Linux ARM64 FUSE refusal; not generic provider qualification',
               'retained_source': str(source), 'fixture_sha256': {name: hashlib.sha256((fixture / name).read_bytes()).hexdigest()
                                  for name in ('fuse_provider.c', 'engine_no_recall_probe.rs', 'run_product_probe.py')}}
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if result.returncode:
        raise RuntimeError(f'private provider run failed: {result.returncode}')
    verify_output((output / 'orchestration.log').read_text(),
                  (output / 'current_product.log').read_text(),
                  (output / 'current_provider.log').read_text())
    shutil.rmtree(source)
    receipt['verified'] = True
    receipt['source_cleanup'] = 'after verified original provider retirement'
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
