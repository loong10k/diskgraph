"""目录租约资格失败时原文件必须逐字恢复；不代替 Windows 原生证明。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_directory_share_retry as qualifier

class SharingRestorationTests(unittest.TestCase):
    def exercise(self, result):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / qualifier.SOURCE
            source.parent.mkdir(parents=True)
            original = b"candidate original\r\n"
            source.write_bytes(original)
            (root / qualifier.TEST).write_bytes(b"shared native test\r\n")
            runner = root / "runner"
            runner.mkdir()
            def git(args, **kwargs):
                return b"original frozen lease\n" if args[1] == "show" else b"candidate\n"
            def cargo(*args):
                self.assertEqual(source.read_bytes(), b"original frozen lease\n")
                if isinstance(result, Exception): raise result
                return result
            with patch.object(qualifier, "ROOT", root), patch.object(qualifier.sys, "platform", "win32"), patch.dict(qualifier.os.environ, {"RUNNER_TEMP": str(runner)}), patch.object(qualifier.subprocess, "run"), patch.object(qualifier.subprocess, "check_output", side_effect=git), patch.object(qualifier, "cargo", side_effect=cargo):
                with self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                    qualifier.main()
            self.assertEqual(source.read_bytes(), original)
            receipt = json.loads((runner / "diskgraph-windows-directory-sharing/receipt.json").read_text())
            self.assertTrue(receipt["restored"])
            self.assertEqual(receipt["status"], "pending")
    def test_compile_failure_is_not_native_red(self): self.exercise((1, "compile failed"))
    def test_unexpected_green_restores(self): self.exercise((0, "1 passed; 0 failed; 0 ignored;"))
    def test_timeout_restores(self): self.exercise(subprocess.TimeoutExpired("cargo", 240))
    def test_exception_restores(self): self.exercise(RuntimeError("runner failed"))
