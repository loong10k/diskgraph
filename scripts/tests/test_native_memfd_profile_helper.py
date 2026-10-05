#!/usr/bin/env python3
"""固定临时规则 helper 的纯装配负控，不代表 kernel profile 或 namespace 验收。

来源：native_memfd_profile 的真实边界；mock 仅 parser/内核视图，文件准入使用真实 OS 对象。
首批 run-path 五案保留原文件和断言，本模块不替代其目标行为。
"""
import importlib.util
import io
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "native_memfd_profile.py"
SPEC = importlib.util.spec_from_file_location("native_memfd_profile_helper", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NativeMemfdProfileHelperContracts(unittest.TestCase):
    """验证固定规则、一次加载责任以及原异常对象与独立清理错误。"""

    def make_profile(self, output):
        failures = []
        supervisor = SimpleNamespace(output=output, receipt={}, pid=None, reaped=False,
                                     streams={}, logs={})
        supervisor.failure = lambda phase, error: failures.append((phase, error))
        profile = MODULE.NativeMemfdProfile(supervisor, time.monotonic() + 20)
        return profile, supervisor, failures

    def parser_stat(self, path, *args, **kwargs):
        if str(path) == MODULE.NativeMemfdProfile.PARSER:
            return SimpleNamespace(st_mode=stat.S_IFREG | 0o755, st_uid=0)
        return self.original_lstat(path, *args, **kwargs)

    def test_actual_symlink_and_fifo_cannot_be_attachment_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary).resolve()
            profile, _, _ = self.make_profile(output)
            profile.fixture.parent.mkdir(parents=True)
            self.original_lstat = os.lstat
            profile.fixture.symlink_to(output)
            with patch.object(MODULE.os, "lstat", side_effect=self.parser_stat):
                with self.assertRaisesRegex(ValueError, "symlink"):
                    profile._check_paths()
                profile.fixture.unlink()
                os.mkfifo(profile.fixture)
                with self.assertRaisesRegex(ValueError, "regular"):
                    profile._check_paths()

    def test_literal_definition_has_no_free_rules_or_descendant_compiler_jobs(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary).resolve()
            profile, _, _ = self.make_profile(output)
            self.assertRegex(profile.name, r"^diskgraph_memfd_policy_[0-9a-f]{32}$")
            self.assertEqual(profile.definition.decode(),
                             "abi <abi/4.0>,\ninclude <tunables/global>\n"
                             f'profile {profile.name} "{profile.fixture}" flags=(unconfined) {{\n  userns,\n}}\n')
            with patch.object(MODULE.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as call:
                profile._step("parse", "--skip-kernel-load")
            argv = call.call_args.args[0]
            self.assertIn("--jobs=0", argv)
            self.assertIn("--config-file=/dev/null", argv)
            self.assertNotIn("--replace", argv)
            self.assertEqual(call.call_args.kwargs["env"],
                             {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C", "LC_ALL": "C"})

    def test_parser_success_without_exact_loaded_witness_does_not_supply_markers(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile, _, _ = self.make_profile(Path(temporary).resolve())
            with patch.object(profile, "require_runner_label", return_value="unconfined"), \
                    patch.object(profile, "_check_paths"), \
                    patch.object(profile, "_read", return_value=b"Y\n"), \
                    patch.object(profile, "_present", return_value=None), \
                    patch.object(profile, "_step") as step:
                with self.assertRaisesRegex(RuntimeError, "exact loaded profile"):
                    profile.load()
            self.assertTrue(profile.add_attempted)
            self.assertFalse(profile.record["loaded_verified"])
            self.assertTrue(profile.record["pending"])
            self.assertEqual([call.args[1] for call in step.call_args_list], ["--skip-kernel-load", "--add"])
            with self.assertRaisesRegex(RuntimeError, "unverified"):
                profile.environment()

    def test_original_parser_timeout_survives_actual_buffer_close_error(self):
        class CloseError(io.BytesIO):
            def close(self):
                super().close()
                raise OSError(5, "actual capture close contract failure")

        with tempfile.TemporaryDirectory() as temporary:
            profile, _, failures = self.make_profile(Path(temporary).resolve())
            original = subprocess.TimeoutExpired([MODULE.NativeMemfdProfile.PARSER], 1)
            streams = [CloseError(), io.BytesIO()]
            with patch.object(MODULE.tempfile, "TemporaryFile", side_effect=streams), \
                    patch.object(MODULE.subprocess, "run", side_effect=original):
                with self.assertRaises(subprocess.TimeoutExpired) as observed:
                    profile._step("load", "--add")
            self.assertIs(observed.exception, original)
            self.assertTrue(all(stream.closed for stream in streams))
            self.assertIs(failures[0][1], original)
            self.assertTrue(any(phase == "memfd_profile_parser_close" and error.errno == 5
                                for phase, error in failures if isinstance(error, OSError)))

    def test_second_capture_creation_failure_closes_first_real_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile, _, _ = self.make_profile(Path(temporary).resolve())
            first = tempfile.TemporaryFile()
            original = OSError(24, "second capture could not open")
            with patch.object(MODULE.tempfile, "TemporaryFile", side_effect=[first, original]), \
                    patch.object(MODULE.subprocess, "run") as call:
                with self.assertRaises(OSError) as observed:
                    profile._step("parse", "--skip-kernel-load")
            self.assertIs(observed.exception, original)
            self.assertTrue(first.closed)
            call.assert_not_called()

    def test_unreaped_init_prevents_unload_and_keeps_owned_responsibility(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile, supervisor, failures = self.make_profile(Path(temporary).resolve())
            profile.add_attempted = True
            profile.record.update(loaded_verified=True, pending=True)
            supervisor.pid = 123
            with patch.object(profile, "_step") as step:
                profile.finish()
            step.assert_not_called()
            self.assertEqual(failures[0][0], "memfd_profile_remove")
            self.assertIn("actual init reap", str(failures[0][1]))
            self.assertTrue(profile.record["pending"])
            self.assertFalse(profile.record["unloaded_verified"])

    def test_remove_success_without_actual_kernel_disappearance_is_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile, supervisor, failures = self.make_profile(Path(temporary).resolve())
            profile.add_attempted = True
            profile.record.update(loaded_verified=True, pending=True)
            supervisor.pid, supervisor.reaped = 123, True
            with patch.object(profile, "_present", return_value=profile.name + " (unconfined)"), \
                    patch.object(profile, "_step") as step:
                profile.finish()
            step.assert_called_once_with("remove", "--remove", cleanup=True)
            self.assertIn("kernel disappearance", str(failures[0][1]))
            self.assertFalse(profile.record["unloaded_verified"])
            self.assertTrue(profile.record["pending"])

    def test_broken_cleanup_witness_still_attempts_owned_rule_removal(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile, supervisor, failures = self.make_profile(Path(temporary).resolve())
            profile.add_attempted = True
            profile.record.update(loaded_verified=True, pending=True)
            supervisor.pid, supervisor.reaped = 123, True
            original = OSError(5, "loaded witness read broke")
            with patch.object(profile, "_present", side_effect=[original, None]), \
                    patch.object(profile, "_step") as step:
                profile.finish()
            step.assert_called_once_with("remove", "--remove", cleanup=True)
            self.assertEqual(failures, [("memfd_profile_remove_witness", original)])
            self.assertTrue(profile.record["unloaded_verified"])
            self.assertFalse(profile.record["pending"])


if __name__ == "__main__":
    unittest.main()
