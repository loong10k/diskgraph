"""部署准备不能伪装为完整平台资格，也不能接受测试缺失或产物替换。"""
import importlib.util
from pathlib import Path
import tempfile
import unittest
import subprocess
import sys

SCRIPT = Path(__file__).resolve().parents[1] / "qualify_macos_installed_worker.py"
SPEC = importlib.util.spec_from_file_location("qualifier_preparation", SCRIPT)
QUALIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(QUALIFIER)


class HostPreparation(unittest.TestCase):
    def test_frozen_source_preparation_is_rejected_before_creating_output(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "must_not_exist"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--prepare-host-only", "--output-dir", str(output)],
                capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 2)
            self.assertIn("frozen source is not permitted", result.stderr)
            self.assertFalse(output.exists())

    def materials(self, root):
        images = [root / name for name in ("source.rs", "helper", "fixture", "driver")]
        for image in images:
            image.write_bytes(image.name.encode())
        manifest = {"sources": {"source.rs": QUALIFIER.digest(images[0])}}
        receipt = {"root_cases_passed": 1, "ordinary_cases_passed": 6,
                   "protocol_cases_passed": 2, "uid": 501,
                   "helper_sha256": QUALIFIER.digest(images[1]),
                   "fixture_sha256": QUALIFIER.digest(images[2]),
                   "driver_fixture": {"sha256": QUALIFIER.digest(images[3])}}
        return manifest, receipt, images[1:]

    def test_completed_preparation_is_explicitly_not_full_qualification(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest, receipt, images = self.materials(root)
            QUALIFIER.finish_host_preparation(root, manifest, receipt, *images)
            self.assertEqual(receipt["status"], "host_prepared")
            self.assertEqual(receipt["acceptance_phase"], "deployment_only")
            self.assertIs(receipt["production_acceptance"], False)

    def test_missing_native_case_cannot_prepare_host(self):
        for field in ("protocol_cases_passed", "root_cases_passed", "ordinary_cases_passed"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                manifest, receipt, images = self.materials(root)
                del receipt[field]
                with self.assertRaises(RuntimeError):
                    QUALIFIER.finish_host_preparation(root, manifest, receipt, *images)

    def test_source_or_binary_replacement_cannot_prepare_host(self):
        for name in ("source.rs", "helper", "fixture", "driver"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                manifest, receipt, images = self.materials(root)
                (root / name).write_bytes(b"replacement")
                with self.assertRaises(RuntimeError):
                    QUALIFIER.finish_host_preparation(root, manifest, receipt, *images)
