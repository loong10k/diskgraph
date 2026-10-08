"""旧目录重试回放的创建依赖与逐字恢复；不代替真实 Windows 重试验收。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_directory_retry as qualifier
from qualify_windows_legacy_api import POOL_DEADLINE_SUPPORT
from qualify_windows_capacity_replay import SUPPORT
from qualify_windows_pool_unwind import ANCHOR, CHECKPOINT

class DirectoryReplayTests(unittest.TestCase):
    def exercise(self, failure):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            originals = {}
            allocation = "crates/diskgraph-engine/src/live_evidence/git_private_allocation.rs"
            for name in dict.fromkeys(qualifier.SOURCES + list(POOL_DEADLINE_SUPPORT) + SUPPORT):
                data = (b"pub(crate) fn drain_until(&self) {}\r\n" if name.endswith("probe_resource_pool.rs")
                        else b"    /// deadline\r\n    pub(crate) fn cleanup_until() {\r\n        actual();\r\n    }\r\noriginal compatibility\r\n" if name in POOL_DEADLINE_SUPPORT else b"current source\r\n")
                if name == SUPPORT[2]:
                    data += (b"    if let Some(capacity) = self.capacity.as_mut() {\n"
                             b"        capacity.recover_created_entry()?;\n    }\n") * 2
                if name == SUPPORT[3]:
                    data = (b"#[cfg(windows)]\nmod git_private_created_entry;\n"
                            b"#[cfg(all(test, windows))]\nmod git_private_write_profile;\n"
                            b"#[cfg(windows)]\nmod windows_git_foreign_removal_witness;\n"
                            b"#[cfg(windows)]\nmod windows_git_root_parent;\n")
                path = root/name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                originals[name] = data
            (root/"crates/diskgraph-engine/src/live_evidence/probe_resource_pool_tests.rs").write_bytes(b"shared tests\n")
            (root/"crates/diskgraph-engine/src/probe_pool_cleanup_fault.rs").write_bytes(b"shared unarmed fault\n")
            runner = root/"runner"
            runner.mkdir()
            def git_output(args, **kwargs):
                if args[1] == "show":
                    return b"before\n"+ANCHOR+b"original compatibility\n" if args[2].endswith("probe_resource_pool.rs") else b"old source\n"
                return b"candidate-revision\n"
            def fail(*args):
                self.assertIn(CHECKPOINT, (root/qualifier.SOURCES[0]).read_bytes())
                self.assertEqual((root/allocation).read_bytes(), b"old source\n",
                                 'old directory creation must compile against its real allocation API')
                for name in POOL_DEADLINE_SUPPORT:
                    self.assertNotIn(b"fn cleanup_until(", (root/name).read_bytes())
                self.assertNotIn(b"recover_created_entry", (root/SUPPORT[2]).read_bytes())
                self.assertNotIn(b"mod git_private_created_entry;", (root/SUPPORT[3]).read_bytes())
                self.assertNotIn(b"mod git_private_write_profile;", (root/SUPPORT[3]).read_bytes())
                self.assertNotIn(b"mod windows_git_foreign_removal_witness;", (root/SUPPORT[3]).read_bytes())
                self.assertNotIn(b"mod windows_git_root_parent;", (root/SUPPORT[3]).read_bytes())
                self.assertEqual((root/POOL_DEADLINE_SUPPORT[1]).read_bytes(), b"old source\n")
                if isinstance(failure, Exception):
                    raise failure
                return failure
            with patch.object(qualifier,"ROOT",root), patch.object(qualifier.sys,"platform","win32"), patch.dict(qualifier.os.environ,{"RUNNER_TEMP":str(runner)}), patch.object(qualifier.subprocess,"run",return_value=SimpleNamespace(returncode=0)), patch.object(qualifier.subprocess,"check_output",side_effect=git_output), patch.object(qualifier,"cargo",side_effect=fail):
                with self.assertRaises((RuntimeError,subprocess.TimeoutExpired)):
                    qualifier.main()
            for name,data in originals.items():
                self.assertEqual((root/name).read_bytes(),data)
            receipt=json.loads((runner/"diskgraph-windows-directory-retry/receipt.json").read_text())
            self.assertTrue(receipt["restored"])
            self.assertEqual(receipt["status"],"pending")
    def test_compile_failure_is_not_behavior_red(self):
        self.exercise((1,"compile failure"))
    def test_unexpected_green_baseline(self):
        self.exercise((0,"2 passed; 0 failed; 0 ignored;"))
    def test_timeout_restores_all_nine_sources(self):
        self.exercise(subprocess.TimeoutExpired("cargo",240))
    def test_exception_restores_all_nine_sources(self):
        self.exercise(RuntimeError("runner failed"))

if __name__ == "__main__":
    unittest.main()
