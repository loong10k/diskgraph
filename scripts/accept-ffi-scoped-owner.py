#!/usr/bin/env python3
"""编译 scoped owner 外部安全 Rust 消费者；只接受本次精确 rlib/Cargo JSON 产物。"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
inputs = parser.add_mutually_exclusive_group(required=True)
inputs.add_argument('--rlib', type=Path, help='本次构建的精确 diskgraph_ffi .rlib，不扫描旧产物')
inputs.add_argument('--cargo-artifacts', type=Path,
                    help='本次成功 cargo build --lib --message-format=json 的完整标准输出')
parser.add_argument('--deps', type=Path, help='显式 --rlib 对应的依赖目录')
parser.add_argument('--output', type=Path, required=True, help='本次探针日志及 metadata 输出目录')
args = parser.parse_args()
repo = Path(__file__).resolve().parent.parent
base = repo / 'crates' / 'diskgraph-ffi' / 'tests' / 'compile_probes'
provenance = {'mode': 'explicit-rlib'}
if args.cargo_artifacts:
    if args.deps:
        parser.error('--cargo-artifacts 不与 --deps 混用；依赖目录取自精确 Cargo 产物所在目录')
    try:
        raw = args.cargo_artifacts.read_bytes()
        messages = [json.loads(line) for line in raw.decode().splitlines() if line.strip()]
        if not messages or any(not isinstance(message, dict) for message in messages):
            parser.error('Cargo 输出必须是完整 JSON 消息流')
        finished = [message for message in messages if message.get('reason') == 'build-finished']
        if len(finished) != 1 or finished[0].get('success') is not True or messages[-1] != finished[0]:
            parser.error('Cargo 输出没有唯一成功的末尾 build-finished')
        manifest = (repo / 'crates' / 'diskgraph-ffi' / 'Cargo.toml').resolve()
        artifacts = [message for message in messages
                     if message.get('reason') == 'compiler-artifact'
                     and message.get('target', {}).get('name') == 'diskgraph_ffi'
                     and 'rlib' in message.get('target', {}).get('crate_types', [])
                     and Path(message.get('manifest_path', '')).resolve() == manifest]
        paths = {Path(name).resolve() for artifact in artifacts
                 for name in artifact.get('filenames', []) if name.endswith('.rlib')}
        if len(artifacts) != 1 or len(paths) != 1:
            parser.error('需要本仓库唯一 diskgraph_ffi rlib 编译产物，拒绝缺失或混合构建记录')
        args.rlib = paths.pop()
        args.deps = args.rlib.parent if args.rlib.parent.name == 'deps' else args.rlib.parent / 'deps'
        provenance = {'mode': 'cargo-artifacts', 'path': str(args.cargo_artifacts.resolve()),
                      'sha256': hashlib.sha256(raw).hexdigest(), 'artifact': artifacts[0]}
    except (OSError, UnicodeError, ValueError, TypeError, AttributeError) as error:
        parser.error(f'无法读取有效 Cargo 产物记录: {error}')
elif args.deps is None:
    parser.error('--rlib 必须同时指定 --deps')
if args.rlib.suffix != '.rlib' or not args.rlib.is_file() or not args.deps.is_dir():
    parser.error('指定 rlib 或对应依赖目录不存在；不会搜索替代产物')
names = [
    'scoped_positive', 'scoped_return_owner', 'scoped_tls_owner',
    'scoped_erased_owner', 'scoped_any_owner', 'scoped_send_owner',
    'scoped_sync_owner', 'scoped_nested_swap',
]
if any(not (base / (name + '.rs')).is_file() for name in names):
    parser.error('仓库中的固定 scoped owner 探针不完整')
args.output.mkdir(parents=True, exist_ok=True)
report = {'phase': 'scoped', 'artifact_provenance': provenance, 'rlib': str(args.rlib.resolve()),
          'rlib_sha256': hashlib.sha256(args.rlib.read_bytes()).hexdigest(),
          'compiler': subprocess.run(['rustc', '--version', '--verbose'], capture_output=True, text=True).stdout,
          'cases': [], 'scope': 'compile only; API absence is not runtime RED'}
passed = True
for index, name in enumerate(names):
    source = base / (name + '.rs')
    command = ['rustc', '--edition=2024', '--crate-type=lib', '--emit=metadata', '--error-format=json',
               '--crate-name', name, str(source), '--extern',
               'diskgraph_ffi=' + str(args.rlib.resolve()), '-L',
               'dependency=' + str(args.deps.resolve()), '-o',
               str((args.output / (name + '.rmeta')).resolve())]
    result = subprocess.run(command, capture_output=True, text=True)
    (args.output / (name + '.stdout')).write_text(result.stdout)
    (args.output / (name + '.stderr')).write_text(result.stderr)
    expected_success = index == 0
    actual = result.returncode == 0
    # 每个反例必须有源位置对应的生命周期或 Send/Sync 诊断；其它失败一律不接受。
    diagnostics = []
    malformed = False
    for line in result.stderr.splitlines():
        try:
            diagnostic = json.loads(line)
            if diagnostic.get('$message_type') != 'diagnostic':
                malformed = True
            else:
                diagnostics.append(diagnostic)
        except (ValueError, AttributeError):
            malformed = True
    errors = [item for item in diagnostics if item.get('level') == 'error'
              and not item.get('message', '').startswith('aborting due to ')]
    classes = []
    for error in errors:
        code = (error.get('code') or {}).get('code')
        message = error.get('message', '')
        rendered = error.get('rendered') or ''
        primary = [span for span in error.get('spans', []) if span.get('is_primary')
                   and Path(span.get('file_name', '')).resolve() == source.resolve()]
        if not primary:
            classes.append('unknown-source')
        elif name in ('scoped_send_owner', 'scoped_sync_owner'):
            transport_error = ('cannot be sent between threads safely' in message
                               or 'cannot be shared between threads safely' in message)
            classes.append('owner-send-sync' if code == 'E0277' and transport_error
                           and 'NativeServiceOwner' in rendered else 'unknown-error')
        else:
            escaped = (code == 'E0521' and message == 'borrowed data escapes outside of closure')
            lifetime = (code is None and message == 'lifetime may not live long enough')
            # 固定 R/不变 brand 的报错必须说明 outlive/escape，且指向本探针 owner 值。
            owner_span = any(any(word in text.get('text', '') for word in ('owner', 'outer', 'inner'))
                             for span in primary for text in span.get('text', []))
            classes.append('owner-lifetime' if (escaped or lifetime) and owner_span
                           and ('outlive' in rendered or 'escapes' in rendered) else 'unknown-error')
    expected_class = 'owner-send-sync' if name in ('scoped_send_owner', 'scoped_sync_owner') else 'owner-lifetime'
    accepted = (actual and not malformed) if expected_success else (
        not actual and not malformed and bool(errors)
        and all(value == expected_class for value in classes)
        and 'internal compiler error' not in result.stderr)
    report['cases'].append({'name': name, 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                            'command': command, 'exit_code': result.returncode,
                            'expected_success': expected_success, 'accepted': accepted,
                            'diagnostic_classes': classes, 'diagnostics': diagnostics,
                            'stdout': result.stdout, 'stderr': result.stderr})
    passed &= accepted
    if index == 0 and not accepted:
        break
report['passed'] = passed
(args.output / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
raise SystemExit(0 if passed else 1)
