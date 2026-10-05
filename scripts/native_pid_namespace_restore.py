"""仅外层 future-children PID namespace 的出生期恢复，不改变原 init owner。

来源：setns(2)/pid_namespaces(7)：NEWPID 不改变 caller 的 active PID namespace，
但 dead init 后未来 fork 可 ENOMEM；原 held namespace FD 必须在 parent 恢复并 readback。
"""
import json
import os
import sys


class NativePidNamespaceRestore:
    """原目的 namespace 的有限 FD 责任；不复制 child、pidfd 或后台回收 owner。"""

    PATH = "/proc/self/ns/pid_for_children"

    def __init__(self, supervisor):
        self.supervisor = supervisor
        self.fd = None
        self.identity = None
        self.armed = False

    def spawn(self, command, parent_fd, pairs):
        """参数：原 command/parent pidfd/三管道；返回：parent 恢复后保留原 init/pidfd。"""
        try:
            self.fd = os.open(self.PATH, os.O_RDONLY | os.O_CLOEXEC)
            original = os.fstat(self.fd)
            self.identity = (original.st_dev, original.st_ino)
            self.supervisor.receipt["pid_for_children"] = {
                "original_device": original.st_dev, "original_inode": original.st_ino,
                "restored_verified": False}
            # unshare/fork 失败也恢复目的 namespace；不建立新期限或移除原异常。
            self.armed = True
            os.unshare(os.CLONE_NEWPID)
            self.supervisor.pid = os.fork()
            if self.supervisor.pid == 0:
                self._child(command, parent_fd, pairs)
            self.supervisor.receipt["init_host_pid"] = self.supervisor.pid
            # 与原 spawn 相同：SIGCHLD 未自动 reap，未 wait 的直系 child 不会复用数值 PID。
            self.supervisor.pidfd = os.pidfd_open(self.supervisor.pid, 0)
            self.supervisor.receipt["init_pidfd_acquired"] = True
        finally:
            if self.supervisor.pid != 0:
                self._finish(sys.exception())

    def _child(self, command, parent_fd, pairs):
        phase = "pid_for_children_child_close"
        try:
            # child 只关保存 FD；绝不 setns 回外层，以免削弱原 PID1 containment。
            os.close(self.fd)
            self.fd = None
            self.armed = False
            phase = "child_pipe_setup"
            for read_fd, _ in pairs.values():
                os.close(read_fd)
            self.supervisor.child(command, parent_fd, pairs["setup"][1],
                                  pairs["stdout"][1], pairs["stderr"][1])
        except BaseException as error:
            record = {"phase": phase, "type": type(error).__name__, "repr": repr(error)[:2048],
                      "errno": getattr(error, "errno", None)}
            try:
                os.write(pairs["setup"][1], json.dumps(record).encode()[:4096] + b"\n")
            except BaseException:
                pass
        # child 准备失败不可落入 outer 的 pid=0 cleanup；内核退出关闭全部余下 FD。
        os._exit(125)

    def _finish(self, original):
        primary = None
        try:
            if self.armed:
                os.setns(self.fd, os.CLONE_NEWPID)
                actual = os.stat(self.PATH)
                if (actual.st_dev, actual.st_ino) != self.identity:
                    raise RuntimeError("pid_for_children restored readback identity mismatch")
                self.supervisor.receipt["pid_for_children"].update(
                    restored_verified=True, restored_device=actual.st_dev, restored_inode=actual.st_ino)
        except BaseException as error:
            if original is None:
                primary = error
            else:
                self.supervisor.failure("pid_for_children_original", original)
                self.supervisor.failure("pid_for_children_restore", error)
        finally:
            if self.fd is not None:
                try:
                    os.close(self.fd)
                except BaseException as error:
                    prior = original if original is not None else primary
                    if prior is None:
                        primary = error
                    else:
                        self.supervisor.failure("pid_for_children_original", prior)
                        self.supervisor.failure("pid_for_children_fd_close", error)
                self.fd = None
        if original is None and primary is not None:
            raise primary
