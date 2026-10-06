"""旧根回放的模块声明/回滚检查；不代替真实 Windows 删除验收。"""
import json
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_root_parent as qualifier


class RootModuleReplayTests(unittest.TestCase):
    def test_only_unused_new_binder_is_removed_and_current_exports_survive(self):
        original = (b"#[cfg(windows)]\r\nmod windows_git_root_parent;\r\n"
                    b"pub(crate) use native_evidence_test_session::NativeEvidenceTestSession;\r\n")
        expected = b"pub(crate) use native_evidence_test_session::NativeEvidenceTestSession;\n"
        self.assertEqual(qualifier.baseline_modules(original), expected)
        self.assertIn(b"NativeEvidenceTestSession", qualifier.baseline_modules(original))

    def test_unknown_or_repeated_module_graph_is_refused(self):
        declaration = b"#[cfg(windows)]\nmod windows_git_root_parent;\n"
        for invalid in [b"unknown module graph", declaration * 2]:
            with self.assertRaises(RuntimeError):
                qualifier.baseline_modules(invalid)


class RootReplayRollbackTests(unittest.TestCase):
    def exercise(self, failure):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = {}
            for name in qualifier.SOURCES:
                content = (b"#[cfg(windows)]\r\nmod windows_git_root_parent;\r\ncurrent exports\r\n"
                           if name.endswith("mod.rs") else b"current behavior\r\n")
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
                originals[name] = content
            for name in ["windows_git_cleanup_tests.rs", "windows_git_root_parent.rs", "windows_git_private_root_tests.rs"]:
                (root / "crates/diskgraph-engine/src/live_evidence" / name).write_bytes(b"shared source\n")
            runner = root / "runner"
            runner.mkdir()
            def git_output(args, **kwargs):
                return b"frozen original behavior\n" if args[1] == "show" else b"candidate-revision\n"
            def fail(*args):
                actual = (root / qualifier.SOURCES[0]).read_bytes()
                self.assertNotIn(b"mod windows_git_root_parent;", actual)
                self.assertIn(b"current exports", actual)
                if isinstance(failure, Exception):
                    raise failure
                return failure
            with patch.object(qualifier, "ROOT", root), patch.object(qualifier.sys, "platform", "win32"), patch.dict(qualifier.os.environ, {"RUNNER_TEMP": str(runner)}), patch.object(qualifier.subprocess, "run"), patch.object(qualifier.subprocess, "check_output", side_effect=git_output), patch.object(qualifier, "cargo", side_effect=fail):
                with self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                    qualifier.main()
            for name, original in originals.items():
                self.assertEqual((root / name).read_bytes(), original)
            receipt = json.loads((runner / "diskgraph-windows-root-parent/receipt.json").read_text())
            self.assertTrue(receipt["restored"])
            self.assertEqual(receipt["status"], "pending")
    def test_compile_failure_does_not_count_as_red(self):
        self.exercise((1, "compile failure"))
    def test_unexpected_baseline_green(self):
        self.exercise((0, "1 passed; 0 failed; 0 ignored;"))
    def test_timeout_restores_all_three_files(self):
        self.exercise(subprocess.TimeoutExpired("cargo", 240))
    def test_exception_restores_all_three_files(self):
        self.exercise(RuntimeError("runner failed"))


if __name__ == "__main__":
    unittest.main()
