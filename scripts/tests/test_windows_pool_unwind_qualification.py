"""原生资格脚本的回滚保障；这些测试不代替 Windows 实际执行。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_pool_unwind as qualifier
from qualify_windows_legacy_api import POOL_DEADLINE_SUPPORT, strip_unused_pool_deadline_method


class QualificationRollbackTests(unittest.TestCase):
    def exercise_failure(self, cargo_failure):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / qualifier.SOURCE
            source.parent.mkdir(parents=True)
            candidate = b"candidate source\r\n"
            source.write_bytes(candidate)
            for name in ["crates/diskgraph-engine/src/lib.rs",
                         "crates/diskgraph-engine/src/probe_pool_cleanup_fault.rs",
                         "crates/diskgraph-engine/src/live_evidence/probe_resource_pool_tests.rs",
                         "crates/diskgraph-engine/src/live_evidence/git_private_directory_owner.rs",
                         "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs"]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"    /// new deadline API\r\n    pub(crate) fn cleanup_until() {\r\n        original_owner();\r\n    }\r\ncompatibility body\r\n" if name in POOL_DEADLINE_SUPPORT else b"shared test support\n")
            baseline = b"before\n" + qualifier.ANCHOR + b"old cleanup\n"
            runner_temp = root / "runner"
            runner_temp.mkdir()

            support_originals = {name: (root / name).read_bytes() for name in POOL_DEADLINE_SUPPORT}

            def git_output(arguments, **kwargs):
                if arguments[1] == "show":
                    return baseline
                return b"candidate-revision\n"

            def fail_cargo(*args):
                self.assertEqual(source.read_bytes(), qualifier.instrument_baseline(baseline))
                for name, data in support_originals.items():
                    self.assertEqual((root / name).read_bytes(), strip_unused_pool_deadline_method(data))
                if isinstance(cargo_failure, Exception):
                    raise cargo_failure
                return cargo_failure

            with patch.object(qualifier, "ROOT", root), \
                 patch.object(qualifier.sys, "platform", "win32"), \
                 patch.dict(qualifier.os.environ, {"RUNNER_TEMP": str(runner_temp)}), \
                 patch.object(qualifier.subprocess, "run"), \
                 patch.object(qualifier.subprocess, "check_output", side_effect=git_output), \
                 patch.object(qualifier, "cargo", side_effect=fail_cargo):
                with self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                    qualifier.main()
            self.assertEqual(source.read_bytes(), candidate, "candidate bytes must be restored exactly")
            for name, data in support_originals.items():
                self.assertEqual((root / name).read_bytes(), data, "all adapted support must be restored exactly")
            receipt = json.loads((runner_temp / "diskgraph-windows-pool-unwind/receipt.json").read_text())
            self.assertTrue(receipt["restored"])
            self.assertEqual(receipt["status"], "pending", "failed qualification cannot claim native acceptance")

    def test_wrong_failure_does_not_accept_red_and_restores_source(self):
        self.exercise_failure((1, "compile failure"))

    def test_unexpected_green_baseline_restores_source(self):
        self.exercise_failure((0, "1 passed; 0 failed; 0 ignored;"))

    def test_subprocess_timeout_restores_source(self):
        self.exercise_failure(subprocess.TimeoutExpired("cargo", 240))

    def test_runner_exception_restores_source(self):
        self.exercise_failure(RuntimeError("runner failed"))

    def test_current_deadline_callers_keep_the_real_old_pool_delegate(self):
        baseline = b"before\n" + qualifier.ANCHOR + b"original cleanup\n"
        candidate = b"pub(crate) fn drain_until(&self) {}"
        actual = qualifier.instrument_baseline(baseline, candidate)
        self.assertTrue(actual.startswith(qualifier.instrument_baseline(baseline)))
        self.assertIn(b"self.drain()", actual)
        self.assertIn(b"DG_LEGACY_RECOVERY_API_DELEGATE_USED=1", actual)
        self.assertEqual(actual.count(b"fn drain_until("), 1)

    def test_checkpoint_instrumentation_rejects_ambiguous_source(self):
        baseline = b"before\r\n" + qualifier.ANCHOR.replace(b"\n", b"\r\n") + b"old\r\n"
        expected = baseline.replace(b"\r\n", b"\n").replace(qualifier.ANCHOR, qualifier.ANCHOR + qualifier.CHECKPOINT)
        self.assertEqual(qualifier.instrument_baseline(baseline), expected)
        for invalid in [b"unknown", qualifier.ANCHOR * 2, expected]:
            with self.assertRaises(RuntimeError):
                qualifier.instrument_baseline(invalid)


if __name__ == "__main__":
    unittest.main()
