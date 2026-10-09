"""冻结基线失败诊断契约；不代替 Linux 原生验收。"""
import hashlib
import importlib.util
from pathlib import Path
import gzip
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('cost', ROOT / 'scripts/accept-linux-scan-namespace-cost.py')
cost = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cost)


class FrozenBaselineDiagnosticTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        fixtures = ROOT / 'scripts/tests/fixtures/frozen_linux_baseline'
        cls.root = gzip.decompress((fixtures / 'linux_scan_namespace.rs.gz').read_bytes())
        cls.native = gzip.decompress((fixtures / 'linux_open.rs.gz').read_bytes())

    def test_diagnostic_is_reversible_to_the_exact_original(self):
        changed = cost.baseline_diagnostic_source(self.root, self.native)
        self.assertNotEqual(changed, self.root)
        for original, diagnostic in cost.BASELINE_DIAGNOSTIC_EDITS:
            self.assertEqual(changed.count(diagnostic.encode()), 1)
            changed = changed.replace(diagnostic.encode(), original.encode())
        self.assertEqual(changed, self.root)

    def test_changed_root_or_native_error_mapping_is_rejected(self):
        for root, native in [(self.root + b'\n', self.native), (self.root, self.native + b'\n')]:
            with self.subTest(root_sha=hashlib.sha256(root).hexdigest()):
                with self.assertRaisesRegex(RuntimeError, 'frozen baseline'):
                    cost.baseline_diagnostic_source(root, native)

    def test_success_path_keeps_cached_open_and_mount_call_order(self):
        changed = cost.baseline_diagnostic_source(self.root, self.native).decode()
        self.assertEqual(changed.count('0x04 | 0x02 | 0x20,'), self.root.decode().count('0x04 | 0x02 | 0x20,'))
        self.assertEqual(changed.count('unique_mount('), self.root.decode().count('unique_mount('))
        self.assertLess(changed.index('DG_BASELINE_HELD_MOUNT_FAILURE'), changed.index('DG_BASELINE_REOPENED_MOUNT_FAILURE'))
        self.assertIn('return Err(Failure::Conflict);', changed)

    def test_only_frozen_baseline_is_modified_and_receipt_binds_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            for path, content in [(cost.BASELINE_ROOT, self.root), (cost.BASELINE_NATIVE, self.native)]:
                target = source / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(content)
            for label, revision in [('candidate', cost.BASELINE_DIAGNOSTIC_REVISION), ('baseline', cost.BASE)]:
                self.assertIsNone(cost.apply_baseline_diagnostic(source, label, revision))
                self.assertEqual((source / cost.BASELINE_ROOT).read_bytes(), self.root)
            receipt = cost.apply_baseline_diagnostic(source, 'baseline', cost.BASELINE_DIAGNOSTIC_REVISION)
            self.assertEqual(receipt['original_sha256'], hashlib.sha256(self.root).hexdigest())
            self.assertEqual(receipt['measured_sha256'], cost.sha(source / cost.BASELINE_ROOT))
            self.assertEqual((source / cost.BASELINE_NATIVE).read_bytes(), self.native)

    def test_drift_rejection_leaves_source_unmodified(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            root = source / cost.BASELINE_ROOT
            root.parent.mkdir(parents=True)
            root.write_bytes(self.root)
            (source / cost.BASELINE_NATIVE).write_bytes(self.native + b'\n')
            with self.assertRaisesRegex(RuntimeError, 'frozen baseline'):
                cost.apply_baseline_diagnostic(source, 'baseline', cost.BASELINE_DIAGNOSTIC_REVISION)
            self.assertEqual(root.read_bytes(), self.root)


if __name__ == '__main__':
    unittest.main()
