#!/usr/bin/env python3
"""现有 child 入口的固定 policy 准备合同；不证明 Linux namespace/sysctl 能力。

所有凭据、mount、sysctl 与 exec 均为明确模型；真实管道仅记录原 child 报错。
首批不导入新 helper 或缺失 API，不把模拟 GLOBAL UID0 写入称为原生验收。
"""
import argparse
from contextlib import ExitStack
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "run-native-pid-namespace.py"
SPEC = importlib.util.spec_from_file_location("memfd_policy_stage_supervisor", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
POLICY = "/proc/sys/vm/memfd_noexec"
CAP_KEYS = ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")


class ModelChildExit(BaseException):
    """仅代替测试进程不可执行的 _exit；不是原生 child 退出证据。"""


class NativeMemfdPolicyStageContracts(unittest.TestCase):
    """从真实已有 child 方法观察 policy 写读、永久降权和 exec 的相对顺序。"""

    def invoke(self, scope, *, floor=0, actual_pid=1, wrong_readback=False, write_error=None):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            args = argparse.Namespace(output_dir=root / "out", runner_uid=1001, runner_gid=1001,
                                      runner_groups=[1001, 27], timeout_seconds=20, self_test=False,
                                      command=[sys.executable, str(SOURCE.with_name("qualify-linux-memfd-execution.py")),
                                               "--output-dir", "{qualification_output}"])
            supervisor = MODULE.NativeNamespaceSupervisor(args)
            # 库内静态 case 选择的输入；默认 child 无此选定 scope，绝不由自由 argv 授权。
            supervisor.policy_scope = scope
            supervisor.receipt["outer_pid_namespace"] = "pid:[model-outer]"
            state = SimpleNamespace(events=[], writes=[], value=floor, uid=0, gid=0,
                                    groups=[], execs=[], error=None, records=[])
            read_fd, setup_fd = os.pipe()
            parent_fd, stdout_fd, stderr_fd = (os.open(os.devnull, os.O_RDWR) for _ in range(3))
            owned = [read_fd, setup_fd, parent_fd, stdout_fd, stderr_fd]
            original_close = os.close
            original_read_text, original_open = Path.read_text, Path.open

            def status():
                zero = "0000000000000000"
                cap = "0000000000200000" if state.uid == 0 else zero
                return "\n".join([*(key + ":\t" + (cap if key in ("CapPrm", "CapEff", "CapBnd") else zero) for key in CAP_KEYS),
                                   "NoNewPrivs:\t1", "Seccomp:\t0"]) + "\n"

            def read_text(path, *args, **kwargs):
                if str(path) == "/proc/self/status":
                    return status()
                if str(path) == POLICY:
                    state.events.append(("policy_read", state.uid))
                    return str(floor if wrong_readback and state.writes else state.value) + "\n"
                return original_read_text(path, *args, **kwargs)

            def write_policy(text):
                state.events.append(("policy_write", state.uid))
                if write_error is not None:
                    raise write_error
                value = int(text.strip())
                if value < floor:
                    raise PermissionError(1, "modeled inherited policy floor")
                state.writes.append(value)
                state.value = value
                return len(text)

            class PolicyStream(io.StringIO):
                def write(self, text):
                    return write_policy(text)

            def path_open(path, mode="r", *args, **kwargs):
                if str(path) == POLICY:
                    if "w" in mode:
                        return PolicyStream()
                    state.events.append(("policy_read", state.uid))
                    return io.StringIO(str(floor if wrong_readback and state.writes else state.value) + "\n")
                return original_open(path, mode, *args, **kwargs)

            def close(fd):
                if fd in owned:
                    owned.remove(fd)
                original_close(fd)

            def setresuid(real, effective, saved):
                self.assertEqual((real, effective, saved), (1001, 1001, 1001))
                state.events.append(("drop_uid", state.uid))
                state.uid = effective

            def setresgid(real, effective, saved):
                self.assertEqual((real, effective, saved), (1001, 1001, 1001))
                state.events.append(("drop_gid", state.uid))
                state.gid = effective

            def syscall(_libc, name, *values):
                state.events.append((name, state.uid))

            def execve(program, command, environment):
                state.events.append(("exec", state.uid))
                state.execs.append((program, command, environment.copy(), state.uid))
                # 不启动任何 native 子进程；原 exec 不返回的事实未由本模型证明。

            def exit_child(code):
                state.events.append(("exit", code))
                raise ModelChildExit()

            libc = SimpleNamespace(prctl=lambda *_: -1)
            poll = SimpleNamespace(register=lambda *_: None, poll=lambda *_: [])
            try:
                with ExitStack() as stack:
                    stack.enter_context(patch.object(MODULE.ctypes, "CDLL", return_value=libc))
                    stack.enter_context(patch.object(MODULE.ctypes, "get_errno", return_value=22))
                    stack.enter_context(patch.object(supervisor, "syscall", side_effect=syscall))
                    stack.enter_context(patch.object(os, "unshare", create=True,
                                                    side_effect=lambda flags: state.events.append(("mount_unshare", state.uid))))
                    stack.enter_context(patch.object(os, "CLONE_NEWNS", 0x20000, create=True))
                    stack.enter_context(patch.object(os, "getpid", return_value=actual_pid))
                    stack.enter_context(patch.object(os, "geteuid", side_effect=lambda: state.uid))
                    stack.enter_context(patch.object(os, "getresuid", create=True,
                                                    side_effect=lambda: (state.uid,) * 3))
                    stack.enter_context(patch.object(os, "getresgid", create=True,
                                                    side_effect=lambda: (state.gid,) * 3))
                    stack.enter_context(patch.object(os, "getgroups", side_effect=lambda: state.groups))
                    stack.enter_context(patch.object(os, "setgroups",
                                                    side_effect=lambda groups: setattr(state, "groups", list(groups))))
                    stack.enter_context(patch.object(os, "setresuid", create=True, side_effect=setresuid))
                    stack.enter_context(patch.object(os, "setresgid", create=True, side_effect=setresgid))
                    stack.enter_context(patch.object(os, "readlink", side_effect=lambda path:
                                                    str(actual_pid) if str(path) == "/proc/self" else "pid:[model-private]"))
                    stack.enter_context(patch.object(Path, "read_text", read_text))
                    stack.enter_context(patch.object(Path, "write_text", lambda path, text, **kwargs:
                                                    write_policy(text) if str(path) == POLICY else self.fail("unexpected policy path")))
                    stack.enter_context(patch.object(Path, "open", path_open))
                    stack.enter_context(patch("select.poll", return_value=poll))
                    stack.enter_context(patch.object(MODULE.pwd, "getpwuid", return_value=
                                                    SimpleNamespace(pw_dir=str(root), pw_name="modeled-runner")))
                    stack.enter_context(patch.object(os, "dup2"))
                    stack.enter_context(patch.object(os, "close", side_effect=close))
                    stack.enter_context(patch.object(os, "execve", side_effect=execve))
                    stack.enter_context(patch.object(os, "_exit", side_effect=exit_child))
                    try:
                        supervisor.child(args.command, parent_fd, setup_fd, stdout_fd, stderr_fd)
                    except ModelChildExit:
                        pass
                if setup_fd in owned:
                    owned.remove(setup_fd)
                    original_close(setup_fd)
                raw = os.read(read_fd, 8192)
                state.records = [json.loads(line) for line in raw.splitlines()]
            finally:
                for fd in owned:
                    original_close(fd)
            return state

    def test_policy_zero_write_readback_precedes_permanent_drop_and_exec(self):
        state = self.invoke(0)
        self.assertEqual(state.writes, [0], "existing child omitted fixed GLOBAL UID0 policy write")
        events = [name for name, _ in state.events]
        self.assertLess(events.index("policy_write"), events.index("drop_uid"))
        self.assertLess(events.index("drop_uid"), events.index("exec"))
        self.assertEqual([uid for name, uid in state.events if name == "policy_write"], [0])
        self.assertEqual(state.execs[0][3], 1001, "runner-writable code cannot execute as root")
        self.assertEqual(state.execs[0][2]["DG_MEMFD_POLICY_INNER"], "0")
        self.assertTrue(any(name == "policy_read" for name, _ in state.events))

    def test_parent_policy_floor_cannot_be_lowered_or_relabelled_passed(self):
        state = self.invoke(0, floor=1)
        self.assertFalse(state.execs, "existing child skipped actual inherited floor preparation")
        self.assertFalse(any(name == "drop_uid" for name, _ in state.events))
        self.assertEqual(state.writes, [])
        self.assertTrue(any(record.get("errno") == 1 for record in state.records))

    def test_successful_write_with_wrong_actual_readback_blocks_exec(self):
        state = self.invoke(1, wrong_readback=True)
        self.assertEqual(state.writes, [1], "must reach actual modeled write before readback rejection")
        self.assertFalse(state.execs, "environment policy scalar cannot replace actual readback")
        self.assertFalse(any(name == "drop_uid" for name, _ in state.events))
        self.assertTrue(any(record.get("phase") == "memfd_policy_setup" for record in state.records))

    def test_non_pid1_refuses_before_policy_write_and_credentials_transition(self):
        state = self.invoke(2, actual_pid=2)
        self.assertFalse(state.writes)
        self.assertFalse(state.execs)
        self.assertFalse(any(name == "drop_uid" for name, _ in state.events),
                         "actual owned namespace qualification must precede root policy preparation")

    def test_policy_write_original_eacces_remains_a_missing_qualification(self):
        state = self.invoke(2, write_error=PermissionError(13, "modeled scoped proc DAC rejection"))
        self.assertFalse(state.execs, "existing child ignored fixed policy preparation failure")
        errors = [record for record in state.records if record.get("errno") == 13]
        self.assertEqual(len(errors), 1)
        self.assertEqual(errors[0]["phase"], "memfd_policy_setup")
        self.assertFalse(any(name == "drop_uid" for name, _ in state.events))


if __name__ == "__main__":
    unittest.main()
