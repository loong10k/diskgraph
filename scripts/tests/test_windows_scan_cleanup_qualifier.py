"""Windows验收驱动的清单与平台边界；不替代原生子进程测试。"""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location("windows_qualifier", SCRIPTS / "qualify_windows_scan_cleanup.py")
qualifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualifier)


class WindowsCleanupQualifierTests(unittest.TestCase):
    def test_exact_native_inventory_rejects_zero_duplicate_or_extra_cases(self):
        valid = list(qualifier.REQUIRED_CASES)
        qualifier.check_cases(valid)
        for invalid in ([], valid[:2], valid + [valid[0]], [valid[0]] * 3,
                        ["native_child::windows::windows_cleanup_recovery_tests::fake"] * 3):
            with self.assertRaises(ValueError):
                qualifier.check_cases(invalid)

    def test_real_frozen_windows_sources_mount_and_match(self):
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory)
            source = SCRIPTS.parent / qualifier.CANDIDATE
            destination = checkout / qualifier.CANDIDATE
            destination.mkdir(parents=True)
            for name in ("candidate.tar.gz", "manifest.json"):
                (destination / name).write_bytes((source / name).read_bytes())
            manifest = qualifier.shared.mount(checkout, qualifier.CANDIDATE)
            self.assertEqual(len(manifest["sources"]), 457)
            qualifier.check_cases(manifest["cleanup_cases"])
            self.assertFalse(manifest["production_cleanup_algorithm_modified"])
            for name, expected in manifest["sources"].items():
                self.assertEqual(qualifier.shared.digest(checkout / name), expected)

    def test_platform_gate_refuses_local_mac_and_non_ci_windows(self):
        for platform, ci, runner in (("Darwin", "true", "github-hosted"),
                                     ("Windows", "false", "github-hosted"),
                                     ("Windows", "true", "self-hosted")):
            with self.assertRaises(RuntimeError):
                qualifier.check_platform(platform, ci, runner)
        qualifier.check_platform("Windows", "true", "github-hosted")


if __name__ == "__main__":
    unittest.main()
