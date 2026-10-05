#!/usr/bin/env python3
"""外层编排的本机合同测试；不模拟 namespace 成功，不充当 Linux 内核验收。"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "run-native-pid-namespace.py"
SPEC = importlib.util.spec_from_file_location("native_pid_namespace", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NativePidNamespaceContracts(unittest.TestCase):
    """验证固定 argv、拒绝未知资格、原异常对象与不丢 owner 的控制分支。"""

    def arguments(self, output):
        return argparse.Namespace(output_dir=Path(output), runner_uid=1000, runner_gid=1000,
                                  runner_groups=[1000, 1001], timeout_seconds=1800, self_test=False,
                                  command=[sys.executable, str(SOURCE), "--output-dir", "{qualification_output}"])

    def test_fixed_argv_replaces_only_separate_output_token(self):
        args = self.arguments("/tmp/native contract ; literal")
        command = MODULE.NativeNamespaceSupervisor.command(args, args.output_dir)
        self.assertEqual(command, [sys.executable, str(SOURCE), "--output-dir",
                                   "/tmp/native contract ; literal/qualification"])
        self.assertEqual(args.command[-1], "{qualification_output}")

    def test_shell_relative_and_duplicate_token_are_rejected(self):
        examples = [["python3", str(SOURCE), "--output-dir", "{qualification_output}"],
                    ["/bin/sh", "-c", "echo bad", "{qualification_output}"],
                    [sys.executable, "relative.py", "--output-dir", "{qualification_output}"],
                    [sys.executable, str(SOURCE), "--output-dir", "prefix{qualification_output}"],
                    [sys.executable, str(SOURCE), "--output-dir", "{qualification_output}", "{qualification_output}"],
                    [sys.executable, str(SOURCE), "--output-dir", "\x00"]]
        for command in examples:
            with self.subTest(command=command):
                args = self.arguments("/tmp/unused")
                args.command = command
                with self.assertRaises(ValueError):
                    MODULE.NativeNamespaceSupervisor.command(args, args.output_dir)

    def test_nonlinux_refuses_before_directory_or_child_creation(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "not-created"
            supervisor = MODULE.NativeNamespaceSupervisor(self.arguments(output))
            with patch.object(MODULE.sys, "platform", "darwin"):
                with self.assertRaisesRegex(RuntimeError, "native Linux"):
                    supervisor.run()
            self.assertFalse(output.exists())
            self.assertIsNone(supervisor.pid)
            self.assertFalse(supervisor.reaped)

    def test_sudo_identity_mismatch_is_not_admitted(self):
        supervisor = MODULE.NativeNamespaceSupervisor(self.arguments("/tmp/not-created"))
        with patch.object(MODULE.sys, "platform", "linux"), \
                patch.object(MODULE.os, "unshare", create=True), \
                patch.object(MODULE.os, "geteuid", return_value=0), \
                patch.dict(os.environ, {"SUDO_UID": "2000", "SUDO_GID": "1000"}):
            with self.assertRaisesRegex(ValueError, "original non-root"):
                supervisor.validate()
        self.assertIsNone(supervisor.pid)

    def test_receipt_write_error_preserves_exact_timeout_object(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "namespace-receipt.json").mkdir()
            supervisor = MODULE.NativeNamespaceSupervisor(self.arguments(output))
            original = TimeoutError("original absolute deadline")
            supervisor.failure("original", original)
            supervisor.save()
            self.assertIs(supervisor.primary, original)
            self.assertEqual(supervisor.receipt["secondary_errors"][0]["phase"], "receipt_write")
            self.assertEqual(supervisor.receipt["status"], "failed")

    def test_finalization_failure_without_primary_is_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "namespace-receipt.json").mkdir()
            supervisor = MODULE.NativeNamespaceSupervisor(self.arguments(output))
            supervisor.save()
            self.assertIsInstance(supervisor.primary, IsADirectoryError)
            self.assertEqual(supervisor.receipt["primary_error"]["phase"], "receipt_write")
            self.assertEqual(supervisor.receipt["status"], "failed")

    def test_interruption_keeps_original_payload_with_multiple_cleanup_errors(self):
        supervisor = MODULE.NativeNamespaceSupervisor(self.arguments("/tmp/unused"))
        original = KeyboardInterrupt("original interrupt")
        supervisor.failure("monitor", original)
        supervisor.failure("signal", PermissionError(1, "signal denied"))
        supervisor.failure("wait", ChildProcessError(10, "not waitable"))
        self.assertIs(supervisor.primary, original)
        self.assertEqual([value["phase"] for value in supervisor.receipt["secondary_errors"]],
                         ["signal", "wait"])
        self.assertFalse(supervisor.reaped)

    def test_wait_error_does_not_mark_or_release_owner(self):
        supervisor = MODULE.NativeNamespaceSupervisor(self.arguments("/tmp/unused"))
        supervisor.pid, supervisor.pidfd = 123, 456
        error = ChildProcessError(10, "original child unavailable")
        with patch.object(MODULE.os, "P_PIDFD", 3, create=True), \
                patch.object(MODULE.os, "WEXITED", 4, create=True), \
                patch.object(MODULE.os, "waitid", side_effect=error, create=True):
            with self.assertRaises(ChildProcessError) as result:
                supervisor.wait_once()
        self.assertIs(result.exception, error)
        self.assertEqual((supervisor.pid, supervisor.pidfd), (123, 456))
        self.assertFalse(supervisor.reaped)
        self.assertNotIn("namespace_descendants_gone", supervisor.receipt)

    def test_repeated_cleanup_failures_are_bounded_without_losing_primary(self):
        supervisor = MODULE.NativeNamespaceSupervisor(self.arguments("/tmp/unused"))
        original = TimeoutError("original")
        supervisor.failure("monitor", original)
        for _ in range(1000):
            supervisor.failure("wait", ChildProcessError(10, "pending"))
        for index in range(100):
            supervisor.failure(f"phase-{index}", OSError(5, "secondary"))
        self.assertIs(supervisor.primary, original)
        self.assertEqual(len(supervisor.receipt["secondary_errors"]), 32)
        self.assertEqual(supervisor.receipt["secondary_errors_suppressed"], 1068)
        self.assertFalse(supervisor.reaped)

    def test_missing_real_setup_never_accepts_a_child_success_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "qualification").mkdir()
            (output / "qualification/receipt.json").write_text(json.dumps(
                {"status": "component_tests_passed_awaiting_outer_cleanup", "executed_parent_cases": 17}))
            supervisor = MODULE.NativeNamespaceSupervisor(self.arguments(output))
            with self.assertRaisesRegex(RuntimeError, "namespace setup failed"):
                supervisor.qualify()
            self.assertFalse(supervisor.reaped)
            self.assertNotIn("namespace_descendants_gone", supervisor.receipt)

    def test_run_primary_survives_final_owner_close_error_and_saves_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            supervisor = MODULE.NativeNamespaceSupervisor(self.arguments(output))
            original = TimeoutError("original before birth")

            def actual_close_error():
                os.close(-1)

            with patch.object(supervisor, "validate", return_value=supervisor.args.command), \
                    patch.object(MODULE.os, "chown"), \
                    patch.object(supervisor, "spawn", side_effect=original), \
                    patch.object(supervisor, "cleanup_owner", side_effect=actual_close_error):
                with self.assertRaises(TimeoutError) as result:
                    supervisor.run()
            self.assertIs(result.exception, original)
            receipt = json.loads((output / "namespace-receipt.json").read_text())
            self.assertEqual(receipt["primary_error"]["type"], "TimeoutError")
            self.assertEqual(receipt["secondary_errors"][0]["phase"], "owner_finalization")
            self.assertEqual(receipt["secondary_errors"][0]["errno"], 9)
            self.assertNotIn("namespace_descendants_gone", receipt)
            self.assertTrue(all(value[0].closed for value in supervisor.logs.values()))

    def test_persistent_selector_error_returns_after_actual_child_wait(self):
        self.run_monitor_probe("select")

    def test_eof_close_error_returns_after_actual_child_wait(self):
        self.run_monitor_probe("eof-close")

    def run_monitor_probe(self, mode):
        try:
            result = subprocess.run([sys.executable, __file__, "--selector-error-probe", mode],
                                    capture_output=True, text=True, timeout=3)
        except subprocess.TimeoutExpired as error:
            self.fail(f"monitor failed to hand off despite actual child wait: {error.stdout!r}")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("ACTUAL_CHILD_WAIT_COMPLETED", result.stdout)
        self.assertIn("ORIGINAL_SELECTOR_ERROR_PRESERVED", result.stdout)


def selector_error_probe(mode):
    """真实普通子进程先 wait；持续实际 EBADF 对象只测编排，不伪装 namespace 资格。"""
    with tempfile.TemporaryDirectory() as temporary:
        case = NativePidNamespaceContracts()
        supervisor = MODULE.NativeNamespaceSupervisor(case.arguments(Path(temporary) / "output"))
        child = subprocess.Popen([sys.executable, "-c", "pass"], stdout=subprocess.DEVNULL)
        read_fd, write_fd = os.pipe()
        selector = MODULE.selectors.DefaultSelector()
        original_close = os.close
        close_error_seen = False
        try:
            try:
                os.read(-1, 1)
            except OSError as error:
                original = error

            def install_child(_command):
                supervisor.pid = child.pid
                supervisor.streams[read_fd] = "stdout"

            def actual_wait():
                if not supervisor.reaped:
                    assert child.wait(timeout=1) == 0
                    supervisor.reaped = True
                    print("ACTUAL_CHILD_WAIT_COMPLETED", flush=True)

            def close_then_real_error(fd):
                nonlocal original, close_error_seen
                if fd == read_fd:
                    try:
                        original_close(fd)
                        original_close(fd)
                    except OSError as actual:
                        assert actual.errno == original.errno
                        if not close_error_seen:
                            original = actual
                            close_error_seen = True
                        raise
                else:
                    original_close(fd)

            if mode == "eof-close":
                original_close(write_fd)
                write_fd = None
            selection = (patch.object(selector, "select", side_effect=original) if mode == "select"
                         else patch.object(MODULE.os, "close", side_effect=close_then_real_error))

            started = time.monotonic()
            with patch.object(supervisor, "validate", return_value=supervisor.args.command), \
                    patch.object(MODULE.os, "chown"), \
                    patch.object(supervisor, "spawn", side_effect=install_child), \
                    patch.object(supervisor, "wait_once", side_effect=actual_wait), \
                    patch.object(MODULE.selectors, "DefaultSelector", return_value=selector), selection:
                try:
                    supervisor.run()
                except OSError as error:
                    assert error is original
                else:
                    raise AssertionError("original selector failure must escape")
            assert time.monotonic() - started < 2
            assert supervisor.reaped and child.returncode == 0
            with case.assertRaises(OSError):
                os.fstat(read_fd)
            receipt = json.loads((supervisor.output / "namespace-receipt.json").read_text())
            assert receipt["status"] == "failed"
            assert receipt["primary_error"]["errno"] == original.errno
            assert "namespace_descendants_gone" not in receipt
            assert all(value[0].closed for value in supervisor.logs.values())
            print("ORIGINAL_SELECTOR_ERROR_PRESERVED", flush=True)
        finally:
            if child.poll() is None:
                child.kill()
            child.wait()
            if write_fd is not None:
                original_close(write_fd)
            selector.close()


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--selector-error-probe":
        selector_error_probe(sys.argv[2])
    else:
        unittest.main()
