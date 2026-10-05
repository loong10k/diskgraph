#!/usr/bin/env python3
"""已有 spawn 路径的 destination 恢复合同；不模拟 Linux namespace 能力通过。

namespace/pidfd syscalls 为明确装配模型，身份 readback 使用真实文件 FD/stat；
普通实际子进程的出生/等待仅证明测试 owner 生命周期，不代替原生 pidfd/PIDns。
来源：setns(2) 只改变未来 children；pid_namespaces(7) 的 dead init ENOMEM。
"""
import argparse
from contextlib import contextmanager, ExitStack
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "run-native-pid-namespace.py"
SPEC = importlib.util.spec_from_file_location("native_pid_namespace_restore", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
CHILDREN_NS = "/proc/self/ns/pid_for_children"
ACTIVE_NS = "/proc/self/ns/pid"
NEWPID = 0x20000000


class NativePidNamespaceRestoreContracts(unittest.TestCase):
    """从实际已有 spawn 消费恢复责任，不导入缺失 helper/API 来制造失败。"""

    @contextmanager
    def route(self, *, restore_error=None, fork_error=None, wrong_readback=False):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            original, isolated, owner_material = (root / name for name in ("original", "isolated", "owner"))
            for path in (original, isolated, owner_material):
                path.write_bytes(path.name.encode())
            args = argparse.Namespace(output_dir=root / "out", runner_uid=1000, runner_gid=1000,
                                      runner_groups=[1000], timeout_seconds=20, self_test=False,
                                      command=[sys.executable, str(SOURCE.with_name("qualify-linux-atomic-launcher.py")),
                                               "--output-dir", "{qualification_output}"])
            supervisor = MODULE.NativeNamespaceSupervisor(args)
            child = subprocess.Popen([sys.executable, "-c",
                                      "import sys; print('ROUTE_CHILD_READY', flush=True); sys.stdin.read()"],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            state = SimpleNamespace(destination=original, events=[], saved_fds=[], pidfds=[], child=child,
                                    original=original, original_identity=(original.stat().st_dev, original.stat().st_ino),
                                    readback_count=0)
            raw_open, raw_stat, raw_pipe = os.open, os.stat, os.pipe

            def namespace_open(path, flags, *args, **kwargs):
                if str(path) == CHILDREN_NS:
                    fd = raw_open(state.destination, flags, *args, **kwargs)
                    if "unshare" not in state.events:
                        state.events.append("capture")
                        state.saved_fds.append(fd)
                    else:
                        state.readback_count += 1
                    return fd
                return raw_open(path, flags, *args, **kwargs)

            def namespace_stat(path, *args, **kwargs):
                if str(path) == CHILDREN_NS:
                    if "restore" in state.events:
                        state.readback_count += 1
                    return raw_stat(state.destination, *args, **kwargs)
                if str(path) == ACTIVE_NS:
                    return raw_stat(original, *args, **kwargs)
                return raw_stat(path, *args, **kwargs)

            def unshare(flags):
                self.assertEqual(flags, NEWPID)
                state.events.append("unshare")
                state.destination = isolated

            def fork():
                state.events.append("fork")
                if fork_error is not None:
                    raise fork_error
                self.assertIsNone(child.poll(), "ordinary child was not live at modeled namespace birth")
                return child.pid

            def pidfd_open(pid, flags):
                self.assertEqual(flags, 0)
                state.events.append("init_pidfd" if pid == child.pid else "parent_pidfd")
                fd = raw_open(owner_material, os.O_RDONLY | os.O_CLOEXEC)
                state.pidfds.append(fd)
                return fd

            def setns(fd, flags):
                self.assertEqual(flags, NEWPID)
                native = os.fstat(fd)
                self.assertEqual((native.st_dev, native.st_ino), state.original_identity,
                                 "restore must use held original FD, not a reopened destination")
                state.events.append("restore")
                if restore_error is not None:
                    raise restore_error
                state.destination = isolated if wrong_readback else original

            def readlink(path, *args, **kwargs):
                if str(path) in (CHILDREN_NS, ACTIVE_NS):
                    selected = original if str(path) == ACTIVE_NS else state.destination
                    return "pid:[" + str(raw_stat(selected).st_ino) + "]"
                return raw_readlink(path, *args, **kwargs)

            def pipe2(_flags):
                pair = raw_pipe()
                for fd in pair:
                    os.set_inheritable(fd, False)
                return pair

            raw_readlink = os.readlink
            try:
                self.assertEqual(child.stdout.readline(), b"ROUTE_CHILD_READY\n")
                with ExitStack() as stack:
                    stack.enter_context(patch.object(MODULE.os, "open", side_effect=namespace_open))
                    stack.enter_context(patch.object(MODULE.os, "stat", side_effect=namespace_stat))
                    stack.enter_context(patch.object(MODULE.os, "readlink", side_effect=readlink))
                    stack.enter_context(patch.object(MODULE.os, "unshare", side_effect=unshare, create=True))
                    stack.enter_context(patch.object(MODULE.os, "fork", side_effect=fork))
                    stack.enter_context(patch.object(MODULE.os, "pidfd_open", side_effect=pidfd_open, create=True))
                    stack.enter_context(patch.object(MODULE.os, "setns", side_effect=setns, create=True))
                    stack.enter_context(patch.object(MODULE.os, "pipe2", side_effect=pipe2, create=True))
                    stack.enter_context(patch.object(MODULE.os, "CLONE_NEWPID", NEWPID, create=True))
                    yield supervisor, state
            finally:
                child.stdin.close()
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
                child.stdout.close()
                child.stderr.close()
                for fd in set(state.saved_fds + state.pidfds + list(supervisor.streams)):
                    try:
                        os.close(fd)
                    except OSError:
                        pass

    def assert_owned(self, supervisor, state):
        self.assertEqual(supervisor.pid, state.child.pid)
        self.assertIsNotNone(supervisor.pidfd)
        os.fstat(supervisor.pidfd)
        self.assertFalse(supervisor.reaped, "restore must not consume the init owner")

    def test_existing_spawn_restores_original_destination_before_return(self):
        with self.route() as (supervisor, state):
            supervisor.spawn(supervisor.args.command)
            self.assert_owned(supervisor, state)
            self.assertIn("capture", state.events, "spawn omitted original pid_for_children FD")
            self.assertIn("restore", state.events, "outer spawn left future forks in the isolated namespace")
            self.assertLess(state.events.index("capture"), state.events.index("unshare"))
            self.assertLess(state.events.index("init_pidfd"), state.events.index("restore"))
            self.assertEqual(state.destination, state.original)
            self.assertGreater(state.readback_count, 0, "setns success alone did not prove actual destination")
            for fd in state.saved_fds:
                with self.assertRaises(OSError):
                    os.fstat(fd)

    def test_actual_init_wait_does_not_leave_cleanup_birth_in_a_dead_destination(self):
        with self.route() as (supervisor, state):
            supervisor.spawn(supervisor.args.command)
            self.assert_owned(supervisor, state)
            state.child.stdin.close()
            self.assertEqual(state.child.wait(timeout=2), 0)
            self.assertEqual(state.destination, state.original,
                             "modeled destination remained in dead init namespace; Linux later fork returns ENOMEM")
            result = subprocess.run([sys.executable, "-c", "print('CLEANUP_BIRTH_READY')"],
                                    capture_output=True, text=True, timeout=2)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "CLEANUP_BIRTH_READY")

    def test_original_setns_denial_preserves_init_and_open_owner_fd(self):
        original = PermissionError(13, "modeled setns restore denial")
        with self.route(restore_error=original) as (supervisor, state):
            with self.assertRaises(PermissionError) as observed:
                supervisor.spawn(supervisor.args.command)
            self.assertIs(observed.exception, original)
            self.assert_owned(supervisor, state)
            self.assertIn("restore", state.events)

    def test_setns_success_with_wrong_readback_fails_without_consuming_init(self):
        with self.route(wrong_readback=True) as (supervisor, state):
            with self.assertRaisesRegex(RuntimeError, "pid_for_children.*(identity|readback|restor)"):
                supervisor.spawn(supervisor.args.command)
            self.assert_owned(supervisor, state)
            self.assertGreater(state.readback_count, 0)

    def test_original_fork_error_restores_destination_without_overwriting_errno(self):
        original = BlockingIOError(11, "modeled fork resource denial")
        with self.route(fork_error=original) as (supervisor, state):
            with self.assertRaises(BlockingIOError) as observed:
                supervisor.spawn(supervisor.args.command)
            self.assertIs(observed.exception, original)
            self.assertIsNone(supervisor.pid)
            self.assertIn("restore", state.events, "fork failure still leaves a future-child destination")
            self.assertEqual(state.destination, state.original)


if __name__ == "__main__":
    unittest.main()
