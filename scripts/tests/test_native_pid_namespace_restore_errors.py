#!/usr/bin/env python3
"""恢复 helper 的次错负控；复用冻结 spawn 场景，不充当 Linux namespace 验收。"""
import importlib.util
import os
from pathlib import Path
import unittest
from unittest.mock import patch


SUPPORT_PATH = Path(__file__).with_name("test_native_pid_namespace_restore.py")
SPEC = importlib.util.spec_from_file_location("native_pid_namespace_restore_errors_support", SUPPORT_PATH)
SUPPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SUPPORT)


class NativePidNamespaceRestoreErrorContracts(unittest.TestCase):
    """原 fork errno 不被恢复/关闭次错盖住；关闭负控产生真实 EBADF 对象。"""

    def test_original_fork_error_retains_restore_error_as_secondary(self):
        primary = BlockingIOError(11, "modeled original fork resource denial")
        secondary = PermissionError(13, "modeled original setns denial")
        case = SUPPORT.NativePidNamespaceRestoreContracts()
        with case.route(fork_error=primary, restore_error=secondary) as (supervisor, state):
            with self.assertRaises(BlockingIOError) as observed:
                supervisor.spawn(supervisor.args.command)
            self.assertIs(observed.exception, primary)
            self.assertIs(supervisor.primary, primary)
            self.assertEqual(supervisor.receipt["primary_error"]["errno"], 11)
            self.assertTrue(any(item["phase"] == "pid_for_children_restore" and item["errno"] == 13
                                for item in supervisor.receipt["secondary_errors"]))
            self.assertIn("restore", state.events)
            self.assertIsNone(supervisor.pid)

    def test_original_fork_error_survives_actual_saved_fd_close_error(self):
        primary = BlockingIOError(11, "modeled original fork resource denial")
        raw_close = os.close
        case = SUPPORT.NativePidNamespaceRestoreContracts()
        with case.route(fork_error=primary) as (supervisor, state):
            def close_then_actual_error(fd):
                raw_close(fd)
                if fd in state.saved_fds:
                    raw_close(fd)  # 同一真实 FD 已关闭，第二次触发 OS EBADF。

            with patch.object(SUPPORT.MODULE.os, "close", side_effect=close_then_actual_error):
                with self.assertRaises(BlockingIOError) as observed:
                    supervisor.spawn(supervisor.args.command)
            self.assertIs(observed.exception, primary)
            self.assertIs(supervisor.primary, primary)
            self.assertEqual(state.destination, state.original)
            self.assertTrue(any(item["phase"] == "pid_for_children_fd_close" and item["errno"] == 9
                                for item in supervisor.receipt["secondary_errors"]))
            self.assertEqual(supervisor.receipt["primary_error"]["errno"], 11)


if __name__ == "__main__":
    unittest.main()
