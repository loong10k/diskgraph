"""固定 PID1 的特权策略准备；只写本进程独占 proc 的一个固定 sysctl。"""
import errno
import os
from pathlib import Path
import time


class NativeMemfdPolicySetup:
    """来源：Linux pid_sysctl.h/proc_sysctl.c，GLOBAL UID0 DAC 与 inherited floor 均保留。"""

    PATH = Path("/proc/sys/vm/memfd_noexec")

    def __init__(self, supervisor):
        self.supervisor = supervisor

    def check_deadline(self):
        end = self.supervisor.policy_deadline_ns
        if end is not None and time.monotonic_ns() >= end:
            raise TimeoutError("original policy case deadline elapsed before root preparation")

    def read(self):
        with self.PATH.open("r") as stream:
            text = stream.read(32)
        if len(text) >= 32 or text.strip() not in ("0", "1", "2"):
            raise ValueError("unknown bounded actual namespace memfd policy")
        return int(text.strip())

    def apply(self):
        """参数：库内静态 scope；返回：实际 PID/proc/策略写读见证，任何失败禁止降权后执行。"""
        scope = self.supervisor.policy_scope
        if type(scope) is not int or scope not in (0, 1, 2):
            raise ValueError("only fixed policy cases zero/one/two are permitted")
        self.check_deadline()
        namespace = os.readlink("/proc/self/ns/pid")
        if (os.getpid() != 1 or os.readlink("/proc/self") != "1"
                or namespace == self.supervisor.receipt.get("outer_pid_namespace")):
            raise RuntimeError("actual new private proc/PID1 qualification missing before policy write")
        if os.geteuid() != 0 or os.getresuid() != (0, 0, 0):
            raise PermissionError("fixed namespace policy setup requires actual GLOBAL UID0")
        inherited = self.read()
        if scope < inherited:
            raise PermissionError(errno.EPERM, "cannot lower inherited namespace memfd policy floor")
        self.check_deadline()
        text = str(scope) + "\n"
        with self.PATH.open("w") as stream:
            if stream.write(text) != len(text):
                raise OSError(errno.EIO, "incomplete fixed namespace policy write")
        actual = self.read()
        if actual != scope:
            raise RuntimeError("actual namespace memfd policy readback mismatch")
        self.check_deadline()
        return {"pid": 1, "proc_self": "1", "pid_namespace": namespace, "setup_uid": 0,
                "fixed_path": str(self.PATH), "inherited_policy": inherited, "actual_policy": actual}
