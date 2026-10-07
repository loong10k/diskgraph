"""验收客户端超时不能冒充原容器退休或丢失原材料定位。"""
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('fuse_lifecycle', ROOT / 'scripts/qualify_linux_fuse_no_recall.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
FIXTURE = ROOT / 'docs/benchmarks/linux_content_no_recall_audit_2026_10_08'


class LifecycleTests(unittest.TestCase):
    """仅模拟 Docker 客户端终态，验证持久状态；不声明实际容器验收。"""

    def exercise(self, outcome):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary).resolve()
            source = base / 'source'
            source.mkdir()
            output = base / 'evidence'
            archive = io.BytesIO()
            with tarfile.open(fileobj=archive, mode='w') as package:
                for name in ('fuse_provider.c', 'engine_no_recall_probe.rs', 'run_product_probe.py'):
                    package.add(FIXTURE / name, arcname=str(FIXTURE.relative_to(ROOT) / name))
                header = tarfile.TarInfo('crates/diskgraph-engine/tests')
                header.type = tarfile.DIRTYPE
                package.addfile(header)

            def run(command, **kwargs):
                # 出生前必须已有原材料及身份定位，不能等客户端返回才记账。
                pending = json.loads((output / 'receipt.json').read_text())
                self.assertEqual(pending['retained_source'], str(source))
                self.assertEqual(len(pending['fixture_sha256']), 3)
                self.assertEqual(command[command.index('--cidfile') + 1], str(output / 'container.cid'))
                if outcome == 'timeout':
                    raise subprocess.TimeoutExpired(command, 1500)
                if outcome == 'valid':
                    for name in ('current_product.log', 'current_provider.log'):
                        (output / name).write_bytes((FIXTURE / name).read_bytes())
                    kwargs['stdout'].write((FIXTURE / 'nested_orchestration.log').read_bytes())
                else:
                    (output / 'current_product.log').write_text('incomplete product evidence')
                    (output / 'current_provider.log').write_text('PROVIDER_MOUNTED')
                return subprocess.CompletedProcess(command, 0)

            with patch.object(sys, 'argv', ['qualifier', '--output-dir', str(output)]), \
                 patch.object(MODULE.platform, 'machine', return_value='aarch64'), \
                 patch.object(MODULE.tempfile, 'mkdtemp', return_value=str(source)), \
                 patch.object(MODULE.subprocess, 'check_output', side_effect=['exact-commit', archive.getvalue()]), \
                 patch.object(MODULE.subprocess, 'run', side_effect=run):
                if outcome == 'timeout':
                    with self.assertRaises(subprocess.TimeoutExpired):
                        MODULE.main()
                elif outcome == 'invalid':
                    with self.assertRaises(RuntimeError):
                        MODULE.main()
                else:
                    MODULE.main()
            receipt = json.loads((output / 'receipt.json').read_text())
            self.assertEqual(receipt['commit'], 'exact-commit')
            self.assertEqual(source.exists(), outcome != 'valid')
            if outcome == 'timeout':
                self.assertIn('unconfirmed', receipt['status'])
                self.assertNotIn('verified', receipt)
            else:
                self.assertEqual(receipt['verified'], outcome == 'valid')

    def test_timeout_retains_original_material_and_identity(self):
        self.exercise('timeout')

    def test_successful_client_with_invalid_evidence_is_not_verified(self):
        self.exercise('invalid')

    def test_only_verified_retirement_allows_source_cleanup(self):
        self.exercise('valid')


if __name__ == '__main__':
    unittest.main()
