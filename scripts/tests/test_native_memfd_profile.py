#!/usr/bin/env python3
"""临时 AppArmor 编排合同；模拟 parser/内核标签，不证明真实 profile 或 namespace。

首批调用已存在的 supervisor.run，验证缺少准备与回收门禁；不导入缺失的新 API。
普通子进程的实际 wait 只证明本测试的先后顺序，不代替 Linux namespace 回收。
"""
import argparse
from contextlib import ExitStack
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "run-native-pid-namespace.py"
SPEC = importlib.util.spec_from_file_location("memfd_profile_supervisor", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NativeMemfdProfileContracts(unittest.TestCase):
    """从既有 run 路径核验 fixed memfd 准备、真实测试 child wait 和卸载次错。"""

    def arguments(self, output):
        return argparse.Namespace(output_dir=output, runner_uid=1000, runner_gid=1000,
                                  runner_groups=[1000], timeout_seconds=20, self_test=False,
                                  command=[sys.executable, str(SOURCE.with_name("qualify-linux-memfd-execution.py")),
                                           "--output-dir", "{qualification_output}"])

    def run_contract(self, output, *, label="unconfined", load_error=None,
                     monitor_error=None, remove_error=None, forbid_spawn=False):
        """替代仅环境资格与 namespace 部分；受测 run/原异常管线本身不替代。"""
        supervisor = MODULE.NativeNamespaceSupervisor(self.arguments(output))
        events, children = [], []
        state = {"name": None, "loaded": False}
        original_open, original_lstat = Path.open, os.lstat

        def controlled_open(path, *args, **kwargs):
            name = str(path)
            if name == "/proc/self/attr/current":
                return io.BytesIO((label + "\n").encode())
            if name == "/sys/module/apparmor/parameters/enabled":
                return io.BytesIO(b"Y\n")
            if name == "/sys/kernel/security/apparmor/profiles":
                value = state["name"] + " (unconfined)\n" if state["loaded"] else ""
                return io.BytesIO(value.encode())
            return original_open(path, *args, **kwargs)

        def controlled_lstat(path, *args, **kwargs):
            if str(path) == "/usr/sbin/apparmor_parser":
                return SimpleNamespace(st_mode=stat.S_IFREG | 0o755, st_uid=0, st_gid=0)
            return original_lstat(path, *args, **kwargs)

        def loader(command, *args, **kwargs):
            events.append(tuple(command))
            text = kwargs.get("input", b"")
            text = text.decode() if isinstance(text, bytes) else text
            found = re.search(r"profile (diskgraph_memfd_policy_[0-9a-f]+) ", text)
            if found:
                state["name"] = found.group(1)
            if "--remove" in command:
                self.assertTrue(supervisor.reaped, "profile removal before actual test child wait")
                self.assertFalse(supervisor.streams, "profile removal before capture FD closure")
                if remove_error is not None:
                    raise remove_error
                state["loaded"] = False
            elif "--add" in command:
                if load_error is not None:
                    raise load_error
                state["loaded"] = True
            return subprocess.CompletedProcess(command, 0)

        def spawn(_command):
            events.append("spawn")
            if forbid_spawn:
                raise AssertionError("run crossed the profile preparation gate")
            child = subprocess.Popen([sys.executable, "-c", "pass"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            children.append(child)
            supervisor.pid = child.pid

        def actual_wait():
            if children and not supervisor.reaped:
                self.assertEqual(children[0].wait(timeout=2), 0)
                supervisor.reaped = True
                events.append("actual_child_wait")

        def monitor(_deadline):
            if monitor_error is not None:
                raise monitor_error
            actual_wait()

        result = None
        try:
            with ExitStack() as stack:
                stack.enter_context(patch.object(supervisor, "validate", return_value=supervisor.args.command))
                stack.enter_context(patch.object(MODULE.os, "chown"))
                stack.enter_context(patch.object(MODULE.signal, "signal"))
                stack.enter_context(patch.object(supervisor, "spawn", side_effect=spawn))
                stack.enter_context(patch.object(supervisor, "monitor", side_effect=monitor))
                stack.enter_context(patch.object(supervisor, "qualify", side_effect=lambda:
                                                 supervisor.receipt.update(status="assembly_contract_only")))
                stack.enter_context(patch.object(supervisor, "cleanup_owner", side_effect=actual_wait))
                stack.enter_context(patch.object(Path, "open", controlled_open))
                stack.enter_context(patch.object(os, "lstat", controlled_lstat))
                stack.enter_context(patch.object(subprocess, "run", side_effect=loader))
                try:
                    supervisor.run()
                except BaseException as error:
                    result = error
        finally:
            for child in children:
                if child.poll() is None:
                    child.kill()
                child.wait()
        return supervisor, events, result

    def test_confined_outer_label_refuses_before_birth(self):
        with tempfile.TemporaryDirectory() as temporary:
            supervisor, events, error = self.run_contract(
                Path(temporary).resolve() / "out", label="restricted-parent (enforce)", forbid_spawn=True)
            self.assertIsInstance(error, PermissionError)
            self.assertIn("unconfined", str(error))
            self.assertNotIn("spawn", events)
            self.assertIsNone(supervisor.pid)

    def test_aare_attachment_path_is_not_a_free_exemption(self):
        with tempfile.TemporaryDirectory() as temporary:
            supervisor, events, error = self.run_contract(Path(temporary).resolve() / "unsafe[glob]")
            self.assertIsInstance(error, ValueError)
            self.assertIn("path", str(error))
            self.assertNotIn("spawn", events)
            self.assertIsNone(supervisor.pid)

    def test_loader_failure_preserves_original_and_prevents_birth(self):
        with tempfile.TemporaryDirectory() as temporary:
            original = PermissionError(13, "actual loader contract denial")
            supervisor, events, error = self.run_contract(Path(temporary).resolve() / "out",
                                                          load_error=original)
            self.assertIs(error, original)
            self.assertNotIn("spawn", events)
            self.assertIsNone(supervisor.pid)
            self.assertEqual(supervisor.receipt["status"], "failed")

    def test_loaded_profile_is_removed_only_after_actual_test_child_wait(self):
        with tempfile.TemporaryDirectory() as temporary:
            supervisor, events, error = self.run_contract(Path(temporary).resolve() / "out")
            self.assertIsNone(error)
            loaded = [index for index, event in enumerate(events)
                      if isinstance(event, tuple) and "--add" in event]
            removed = [index for index, event in enumerate(events)
                       if isinstance(event, tuple) and "--remove" in event]
            self.assertEqual(len(loaded), 1, "memfd run omitted temporary profile load")
            self.assertEqual(len(removed), 1, "memfd run omitted temporary profile removal")
            self.assertLess(loaded[0], events.index("spawn"))
            self.assertLess(events.index("actual_child_wait"), removed[0])
            self.assertTrue(supervisor.receipt["memfd_profile"]["unloaded_verified"])

    def test_original_timeout_survives_profile_remove_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            original = TimeoutError("original namespace request deadline")
            secondary = OSError(5, "profile removal failed")
            supervisor, events, error = self.run_contract(Path(temporary).resolve() / "out",
                                                          monitor_error=original, remove_error=secondary)
            self.assertIs(error, original)
            self.assertTrue(supervisor.reaped)
            self.assertIn("actual_child_wait", events)
            errors = supervisor.receipt["secondary_errors"]
            self.assertTrue(any(item["phase"] == "memfd_profile_remove" and item["errno"] == 5
                                for item in errors), "unload failure was omitted after original timeout")
            self.assertEqual(supervisor.receipt["status"], "failed")
            self.assertFalse(supervisor.receipt["memfd_profile"].get("unloaded_verified", False))


if __name__ == "__main__":
    unittest.main()
