"""期限资格的故障恢复测试；不代替 Windows 原生执行。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_probe_deadline as qualifier
from qualify_windows_legacy_api import POOL_DEADLINE_SUPPORT, strip_unused_pool_deadline_method

class DeadlineRollbackTests(unittest.TestCase):
    def exercise(self, failure):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / qualifier.SOURCE
            source.parent.mkdir(parents=True)
            candidate = b"pub(crate) fn drain_until(&self) {}\r\n"
            source.write_bytes(candidate)
            shared = ["crates/diskgraph-engine/src/probe_recovery.rs",
                      "crates/diskgraph-engine/src/live_evidence/probe_resource_pool_tests.rs",
                      "crates/diskgraph-engine/src/live_evidence/git_private_directory_owner.rs",
                      "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs",
                      "scripts/qualify_windows_legacy_api.py"]
            for name in shared:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"    /// new deadline API\r\n    pub(crate) fn cleanup_until() {\r\n        original_owner();\r\n    }\r\ncompatibility body\r\n" if name in POOL_DEADLINE_SUPPORT else b"shared source\n")
            runner = root / "runner"
            runner.mkdir()
            baseline = b"actual legacy pool source\n"
            support_originals = {name: (root / name).read_bytes() for name in POOL_DEADLINE_SUPPORT}

            def git_output(args, **kwargs):
                return baseline if args[1] == "show" else b"candidate-revision\n"
            def fail(*args):
                self.assertEqual(source.read_bytes(), qualifier.bridge_baseline(candidate, baseline, "pool"))
                for name, data in support_originals.items():
                    self.assertEqual((root / name).read_bytes(), strip_unused_pool_deadline_method(data))
                if isinstance(failure, Exception):
                    raise failure
                return failure
            with patch.object(qualifier, "ROOT", root), patch.object(qualifier.sys, "platform", "win32"), patch.dict(qualifier.os.environ, {"RUNNER_TEMP": str(runner)}), patch.object(qualifier.subprocess, "run"), patch.object(qualifier.subprocess, "check_output", side_effect=git_output), patch.object(qualifier, "cargo", side_effect=fail):
                with self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                    qualifier.main()
            self.assertEqual(source.read_bytes(), candidate)
            for name, data in support_originals.items():
                self.assertEqual((root / name).read_bytes(), data, "all adapted support must be restored exactly")
            receipt = json.loads((runner / "diskgraph-windows-probe-deadline/receipt.json").read_text())
            self.assertTrue(receipt["restored"])
            self.assertEqual(receipt["status"], "pending")
    def test_compile_failure_is_not_red(self):
        self.exercise((1, "compile failure"))
    def test_unexpected_baseline_green(self):
        self.exercise((0, "1 passed; 0 failed; 0 ignored;"))
    def test_timeout_restores_candidate(self):
        self.exercise(subprocess.TimeoutExpired("cargo", 240))
    def test_exception_restores_candidate(self):
        self.exercise(RuntimeError("runner failed"))

if __name__ == "__main__":
    unittest.main()
