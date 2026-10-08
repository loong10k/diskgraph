"""冻结外来项回放必须保留当前测试导出，且失败时逐字恢复。"""
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_enumerated_absence as qualifier
from qualify_windows_capacity_replay import SUPPORT

class AbsenceModuleTests(unittest.TestCase):
    def test_preserves_exports_and_removes_only_unreachable_new_modules(self):
        source = (b"#[cfg(windows)]\r\nmod windows_git_root_parent;\r\n"
                  b"#[cfg(windows)]\r\nmod windows_git_foreign_removal_witness;\r\n"
                  b"pub(crate) use native_evidence_test_session::NativeEvidenceTestSession;\r\n")
        self.assertEqual(qualifier.baseline_modules(source),
            b"pub(crate) use native_evidence_test_session::NativeEvidenceTestSession;\n")

    def test_refuses_missing_or_duplicate_declarations(self):
        root = b"#[cfg(windows)]\nmod windows_git_root_parent;\n"
        foreign = b"#[cfg(windows)]\nmod windows_git_foreign_removal_witness;\n"
        for source in (root, foreign, root + root + foreign):
            with self.assertRaises(RuntimeError):
                qualifier.baseline_modules(source)

import json
import subprocess
import tempfile
from unittest.mock import patch

class AbsenceRollbackTests(unittest.TestCase):
    def exercise(self, result):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = {}
            for name in qualifier.SOURCES:
                data = (b"#[cfg(windows)]\r\nmod windows_git_root_parent;\r\n"
                        b"#[cfg(windows)]\r\nmod windows_git_foreign_removal_witness;\r\n"
                        b"current exports\r\n" if name.endswith("mod.rs") else b"current source\r\n")
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                originals[name] = data
            # 新创建恢复已从旧容量回放中不可达；测试夹具仍须覆盖全部恢复文件。
            for name in SUPPORT:
                data = (originals[name] + b"#[cfg(windows)]\nmod git_private_created_entry;\n"
                        b"#[cfg(all(test, windows))]\nmod git_private_write_profile;\n"
                        if name.endswith("mod.rs") else (qualifier.ROOT / name).read_bytes())
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                originals[name] = data
            for name in ("windows_git_foreign_removal_witness", "windows_git_root_parent",
                         "windows_git_directory_cursor_tests", "windows_git_cleanup_tests"):
                (root / f"crates/diskgraph-engine/src/live_evidence/{name}.rs").write_bytes(b"shared\n")
            runner = root / "runner"
            runner.mkdir()
            def git(args, **kwargs):
                return b"frozen old source\n" if args[1] == "show" else b"candidate\n"
            def cargo(*args):
                for name in originals:
                    self.assertNotEqual((root / name).read_bytes(), originals[name])
                module = (root / qualifier.SOURCES[3]).read_bytes()
                self.assertIn(b"current exports", module)
                self.assertNotIn(b"mod windows_git_root_parent;", module)
                if isinstance(result, Exception):
                    raise result
                return result
            with patch.object(qualifier, "ROOT", root), patch.object(qualifier.sys, "platform", "win32"), patch.dict(qualifier.os.environ, {"RUNNER_TEMP": str(runner)}), patch.object(qualifier.subprocess, "run"), patch.object(qualifier.subprocess, "check_output", side_effect=git), patch.object(qualifier, "cargo", side_effect=cargo):
                with self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                    qualifier.main()
            for name, data in originals.items():
                self.assertEqual((root / name).read_bytes(), data)
            receipt = json.loads((runner / "diskgraph-windows-enumerated-absence/receipt.json").read_text())
            self.assertTrue(receipt["restored"])
            self.assertEqual(receipt["status"], "pending")

    def test_compile_error_is_not_behavior_red(self):
        self.exercise((1, "compile failed"))
    def test_unexpected_green_restores(self):
        self.exercise((0, "1 passed; 0 failed; 0 ignored;"))
    def test_timeout_restores(self):
        self.exercise(subprocess.TimeoutExpired("cargo", 240))
    def test_exception_restores(self):
        self.exercise(RuntimeError("failure"))
    def test_legacy_delegate_execution_is_refused(self):
        self.exercise((1, qualifier.MARKER + "0 passed; 1 failed; 0 ignored; DG_WINDOWS_ENUMERATED_ABSENCE_RED_READY=1 full-ID absence must retire only vanished foreign entry:"))
