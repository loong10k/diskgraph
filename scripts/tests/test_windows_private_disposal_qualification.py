"""私有证据回放拒绝编译失败冒充 RED，并在所有路径恢复源码。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_private_disposal as qualifier
from qualify_windows_capacity_replay import BUDGET, SUPPORT


class DisposalReplayTests(unittest.TestCase):
    def exercise(self, result):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = {}
            for name in dict.fromkeys(qualifier.SOURCES + SUPPORT + [BUDGET]):
                data = (qualifier.ROOT / name).read_bytes()
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                originals[name] = data
            for name in ("windows_git_directory_cursor_tests", "git_callback_tests"):
                (root / f"crates/diskgraph-engine/src/live_evidence/{name}.rs").write_bytes(b"shared\n")
            runner = root / "runner"
            runner.mkdir()

            def git(args, **kwargs):
                return b"original legacy source\n" if args[1] == "show" else b"candidate\n"

            def cargo(*args):
                self.assertNotIn(b"recover_created_entry", (root / SUPPORT[2]).read_bytes())
                self.assertNotIn(b"git_private_created_entry", (root / SUPPORT[3]).read_bytes())
                self.assertNotIn(b"git_private_write_summary", (root / SUPPORT[3]).read_bytes())
                self.assertNotIn(b"_write_summary", (root / BUDGET).read_bytes())
                if isinstance(result, Exception):
                    raise result
                return result

            with patch.object(qualifier, "ROOT", root), patch.object(qualifier.sys, "platform", "win32"), patch.dict(qualifier.os.environ, {"RUNNER_TEMP": str(runner)}), patch.object(qualifier.subprocess, "run"), patch.object(qualifier.subprocess, "check_output", side_effect=git), patch.object(qualifier, "cargo", side_effect=cargo):
                with self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                    qualifier.main()
            for name, data in originals.items():
                self.assertEqual((root / name).read_bytes(), data)
            receipt = json.loads((runner / "diskgraph-windows-private-disposal/receipt.json").read_text())
            self.assertTrue(receipt["restored"])
            self.assertEqual(receipt["status"], "pending")

    def test_compile_failure_is_not_behavior_red(self):
        self.exercise((1, "compile failed E0599"))

    def test_unexpected_green_is_rejected(self):
        self.exercise((0, "1 passed; 0 failed; 0 ignored;"))

    def test_timeout_restores_all_support_sources(self):
        self.exercise(subprocess.TimeoutExpired("cargo", 240))

    def test_exception_restores_all_support_sources(self):
        self.exercise(RuntimeError("runner failed"))


if __name__ == "__main__":
    unittest.main()
