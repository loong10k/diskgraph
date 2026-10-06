#!/usr/bin/env python3
"""从当前 Cargo bin artifact 固定 Linux 产品验收镜像，不向远程请求授予信任。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess


def snapshot_worker(rows, destination):
    """独占复制原句柄字节并记录摘要；后续编译不能覆盖验收使用的副本。"""
    artifacts = [row for row in rows if row.get('reason') == 'compiler-artifact'
                 and row.get('target', {}).get('name') == 'diskgraph-scan-worker'
                 and row.get('target', {}).get('kind') == ['bin'] and row.get('executable')]
    if len(artifacts) != 1:
        raise ValueError('one actual Cargo scan worker binary artifact is required')
    image = Path(artifacts[0]['executable'])
    if not image.is_absolute() or not destination.is_absolute() or any(
            char in str(path) for path in (image, destination) for char in '\r\n\0'):
        raise ValueError('invalid worker artifact or destination path')
    fd = os.open(image, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, 'rb') as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode) or not 0 < before.st_size <= 128 * 1024 * 1024:
            raise ValueError('worker must be a bounded regular image')
        digest = hashlib.sha256()
        remaining = before.st_size
        # 独占创建；副本路径不指向可被后续 Cargo 重写的 target 文件。
        with destination.open('xb') as output:
            while remaining:
                block = source.read(min(65536, remaining))
                if not block:
                    raise ValueError('worker changed during deployment copy')
                output.write(block)
                digest.update(block)
                remaining -= len(block)
            after = os.fstat(source.fileno())
            fields = ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')
            if any(getattr(before, field) != getattr(after, field) for field in fields):
                raise ValueError('worker changed during deployment copy')
            output.flush()
            os.fchmod(output.fileno(), 0o500)
    return {'worker_path': str(destination), 'cargo_artifact_path': str(image),
            'worker_sha256': digest.hexdigest(), 'worker_bytes': before.st_size,
            'trust_source': 'current CI checkout Cargo binary artifact snapshot',
            'product_acceptance': 'pending'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args()
    rows = [json.loads(line) for line in args.artifacts.read_text().splitlines()]
    receipt = snapshot_worker(rows, args.output_dir / 'product-scan-worker')
    receipt['checkout_sha'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    (args.output_dir / 'product-worker-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    with Path(os.environ['GITHUB_ENV']).open('a', encoding='utf-8') as output:
        for key, value in [('PATH', receipt['worker_path']), ('SHA256', receipt['worker_sha256']),
                           ('BYTES', receipt['worker_bytes'])]:
            output.write(f'DISKGRAPH_SCAN_WORKER_{key}={value}\n')


if __name__ == '__main__':
    main()
