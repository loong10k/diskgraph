"""在隔离CI checkout中核对并装配当前扫描宿主源码，不改上游扫描器。"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess


def assemble(root, output):
    root = root.resolve()
    candidate = root / 'crates/diskgraph-engine/integration_candidates/scan_worker_host'
    manifest = json.loads((candidate / 'current.json').read_text(encoding='utf-8'))
    patch = candidate / 'current.diff'
    if manifest['schema_version'] != 1 or hashlib.sha256(patch.read_bytes()).hexdigest() != manifest['patch_sha256']:
        raise ValueError('candidate patch identity mismatch')
    entries = manifest['files']
    paths = set()
    for entry in entries:
        path = PurePosixPath(entry['path'])
        if path.is_absolute() or '..' in path.parts or str(path) in paths:
            raise ValueError('invalid or duplicate candidate path')
        if str(path) != 'Cargo.lock' and not str(path).startswith(('crates/diskgraph-cli/', 'crates/diskgraph-engine/', 'crates/diskgraph-mcp/')):
            raise ValueError('candidate outside host integration')
        paths.add(str(path))
        target = root / path
        if any(parent.is_symlink() for parent in [target, *target.parents]):
            raise ValueError('candidate path has symlink')
        before = hashlib.sha256(target.read_bytes()).hexdigest() if target.exists() else None
        if before != entry['before_sha256']:
            raise ValueError('candidate input identity mismatch: ' + str(path))
    statistics = subprocess.check_output(['git', 'apply', '--numstat', str(patch)], cwd=root, text=True, timeout=20)
    changed = [line.split('\t', 2)[2] for line in statistics.splitlines()]
    if len(changed) != len(paths) or set(changed) != paths:
        raise ValueError('patch and manifest path sets differ')
    subprocess.run(['git', 'apply', '--check', str(patch)], cwd=root, check=True, timeout=20)
    subprocess.run(['git', 'apply', str(patch)], cwd=root, check=True, timeout=20)
    for entry in entries:
        if hashlib.sha256((root / entry['path']).read_bytes()).hexdigest() != entry['after_sha256']:
            raise ValueError('candidate output identity mismatch: ' + entry['path'])
    output.mkdir(parents=True, exist_ok=True)
    (output / 'assembled-source.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    print('Verified current host source files:', len(entries))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    assemble(Path.cwd(), args.output)
