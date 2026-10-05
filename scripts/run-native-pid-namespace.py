#!/usr/bin/env python3
"""原用户执行原生资格；外层持有 PID namespace init，真实 wait 后才声明回收。

依据 Linux v6.12 kernel/pid_namespace.c:zap_pid_ns_processes：init 被 reap 时
该 namespace 后代已经退出。此证据不替代 17 个 C8 case 的原 pidfd 验收。
sudo 仅用于创建 namespace/mount 和恢复凭据；Cargo 不以 root 运行。
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import runpy
import selectors
import signal
import stat
import sys
import time
import pwd


# 受限编排 profile；不能由调用者或子 receipt 提供期望数量。
PROFILES = {"qualify-linux-atomic-launcher.py": 17, "qualify-linux-memfd-execution.py": 6}

class NativeNamespaceSupervisor:
    """唯一外层 init owner；日志和 receipt 失败不得覆盖原资格或退出错误。"""

    def __init__(self, args):
        self.args = args
        self.output = args.output_dir.absolute()
        self.receipt = {"schema_version": 1, "status": "incomplete",
                        "scope": "outer namespace containment, not C8 child wait proof",
                        "kernel": os.uname().release, "secondary_errors": [],
                        "secondary_errors_suppressed": 0}
        self.primary = None
        self.pid = None
        self.pidfd = None
        self.reaped = False
        self.streams = {}
        self.logs = {}
        self.setup = bytearray()
        self.witness = bytearray()
        self.memfd_profile = None

    @staticmethod
    def command(args, output):
        """验证固定 argv；不调用 shell，也不推断 PATH 中的解释器。"""
        command = list(args.command)
        if command and command[0] == "--":
            command.pop(0)
        if (len(command) != 4 or not os.path.isabs(command[0]) or
                os.path.realpath(command[0]) != os.path.realpath(sys.executable) or
                not os.path.isabs(command[1]) or not command[1].endswith(".py") or
                command[2] != ("--descendant-fixture" if args.self_test else "--output-dir")):
            raise ValueError("fixed absolute Python/script argv is required; no shell or PATH lookup")
        if any("\x00" in value for value in command):
            raise ValueError("NUL in argv")
        if command.count("{qualification_output}") != 1:
            raise ValueError("exactly one separate {qualification_output} argument is required")
        if args.self_test:
            if Path(command[1]).resolve() != Path(__file__).resolve():
                raise ValueError("self-test must use this fixed supervisor fixture")
        elif Path(command[1]).name not in PROFILES:
            raise ValueError("unknown fixed native qualification profile")
        return [str(output / "qualification") if value == "{qualification_output}" else value
                for value in command]

    def failure(self, phase, error):
        """保存第一个异常原对象；后续错误独立记账。"""
        self.receipt["status"] = "failed"
        first = self.primary is None
        if first:
            self.primary = error
        elif (len(self.receipt["secondary_errors"]) >= 32 or
              any(item["phase"] == phase for item in self.receipt["secondary_errors"])):
            self.receipt["secondary_errors_suppressed"] += 1
            return
        try:
            diagnostic = repr(error)[:2048]
        except BaseException:
            diagnostic = "exception repr unavailable; original object retained"
        item = {"phase": phase, "type": type(error).__name__, "repr": diagnostic}
        if isinstance(error, OSError):
            item["errno"] = error.errno
        if first:
            self.receipt["primary_error"] = item
        else:
            self.receipt["secondary_errors"].append(item)

    def save(self):
        try:
            (self.output / "namespace-receipt.json").write_text(
                json.dumps(self.receipt, indent=2) + "\n")
        except BaseException as error:
            self.failure("receipt_write", error)

    def validate(self):
        if sys.platform != "linux" or not hasattr(os, "unshare"):
            raise RuntimeError("native Linux and Python os.unshare are required; not a skip")
        if os.geteuid() != 0:
            raise PermissionError("sudo namespace setup is required")
        if (str(self.args.runner_uid) != os.environ.get("SUDO_UID") or
                str(self.args.runner_gid) != os.environ.get("SUDO_GID") or
                self.args.runner_uid <= 0 or self.args.runner_gid < 0):
            raise ValueError("explicit runner identity must equal the original non-root sudo identity")
        if self.args.runner_gid not in self.args.runner_groups or any(
                value < 0 for value in self.args.runner_groups):
            raise ValueError("original supplementary groups must include runner gid")
        if not 0 < self.args.timeout_seconds <= 1800:
            raise ValueError("absolute run window must be at most 1800 seconds")
        command = self.command(self.args, self.output)
        mode = os.stat(command[0]).st_mode
        if not stat.S_ISREG(mode) or mode & (stat.S_ISUID | stat.S_ISGID):
            raise ValueError("executable must be regular and not set-id")
        return command

    @staticmethod
    def syscall(libc, name, *args):
        if getattr(libc, name)(*args) == -1:
            code = ctypes.get_errno()
            raise OSError(code, f"{name}: {os.strerror(code)}")

    def child(self, command, parent_fd, setup_fd, stdout_fd, stderr_fd):
        """PID1 先建立私有 procfs，再永久降权；exec 前输出实际资格。"""
        phase = "child_start"
        try:
            libc = ctypes.CDLL(None, use_errno=True)
            # 凭据变化会清 PDEATHSIG，因此前后均设置；原 parent pidfd 防死亡竞态。
            self.syscall(libc, "prctl", 1, signal.SIGKILL, 0, 0, 0)
            phase = "mount_namespace"
            os.unshare(os.CLONE_NEWNS)
            self.syscall(libc, "mount", None, b"/", None, (1 << 14) | (1 << 18), None)
            self.syscall(libc, "mount", b"proc", b"/proc", b"proc", 2 | 4 | 8, None)
            phase = "drop_credentials"
            # 清 bounding 与 ambient；setresuid 后有效/许可/inheritable 再显式清零。
            index = 0
            while True:
                present = libc.prctl(23, index, 0, 0, 0)
                if present == -1:
                    if ctypes.get_errno() == 22:
                        break
                    raise OSError(ctypes.get_errno(), "PR_CAPBSET_READ")
                self.syscall(libc, "prctl", 24, index, 0, 0, 0)
                index += 1
                if index > 1024:
                    raise RuntimeError("capability enumeration exceeded bound")
            self.syscall(libc, "prctl", 47, 4, 0, 0, 0)
            os.setgroups(self.args.runner_groups)
            os.setresgid(self.args.runner_gid, self.args.runner_gid, self.args.runner_gid)
            os.setresuid(self.args.runner_uid, self.args.runner_uid, self.args.runner_uid)
            header = (ctypes.c_uint32 * 2)(0x20080522, 0)
            data = (ctypes.c_uint32 * 6)()
            self.syscall(libc, "capset", ctypes.byref(header), ctypes.byref(data))
            self.syscall(libc, "prctl", 38, 1, 0, 0, 0)
            self.syscall(libc, "prctl", 1, signal.SIGKILL, 0, 0, 0)
            import select
            check = select.poll()
            check.register(parent_fd, select.POLLIN)
            if check.poll(0):
                raise RuntimeError("outer owner already exited")
            phase = "identity_witness"
            status = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines()
                          if ":" in line)
            caps = {key: status[key].strip() for key in ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")}
            if any(int(value, 16) for value in caps.values()):
                raise RuntimeError("nonzero runner capabilities")
            if os.getpid() != 1 or os.readlink("/proc/self") != "1":
                raise RuntimeError("not the init of the mounted PID namespace")
            identity = {"phase": "ready", "pid": 1, "pid_namespace": os.readlink("/proc/self/ns/pid"),
                        "uid": list(os.getresuid()), "gid": list(os.getresgid()),
                        "groups": sorted(os.getgroups()), "capabilities": caps,
                        "no_new_privs": status["NoNewPrivs"].strip(), "seccomp": status["Seccomp"].strip()}
            if identity["no_new_privs"] != "1":
                raise RuntimeError("no_new_privs not established")
            if (identity["uid"] != [self.args.runner_uid] * 3 or
                    identity["gid"] != [self.args.runner_gid] * 3 or
                    identity["groups"] != sorted(self.args.runner_groups)):
                raise RuntimeError("runner credentials did not match original declared identity")
            if self.memfd_profile is not None:
                identity["apparmor_label"] = self.memfd_profile.require_runner_label()
            os.write(setup_fd, json.dumps(identity).encode() + b"\n")
            os.dup2(stdout_fd, 1)
            os.dup2(stderr_fd, 2)
            for fd in (parent_fd, setup_fd, stdout_fd, stderr_fd):
                if fd != setup_fd:
                    os.close(fd)
            environment = os.environ.copy()
            for key in list(environment):
                if key.startswith(("SUDO_", "LD_", "DYLD_", "PYTHON")):
                    environment.pop(key, None)
            account = pwd.getpwuid(self.args.runner_uid)
            environment.update(HOME=account.pw_dir, USER=account.pw_name, LOGNAME=account.pw_name,
                               DG_NATIVE_PID_NAMESPACE=identity["pid_namespace"],
                               DG_NATIVE_NAMESPACE_SUPERVISED="1", DG_NATIVE_NAMESPACE_OUTPUT=str(self.output))
            if self.memfd_profile is not None:
                environment.update(self.memfd_profile.environment())
            phase = "exec"
            # setup_fd保留CLOEXEC，exec失败仍可报告真实errno，成功时自动EOF。
            os.execve(command[0], command, environment)
        except BaseException as error:
            record = {"phase": phase, "type": type(error).__name__, "repr": repr(error),
                      "errno": getattr(error, "errno", None)}
            try:
                os.write(setup_fd, json.dumps(record).encode()[:4096] + b"\n")
            except BaseException:
                pass
            os._exit(125)

    def spawn(self, command):
        self.receipt["outer_pid_namespace"] = os.readlink("/proc/self/ns/pid")
        pairs = {}
        parent_fd = None
        try:
            for name in ("setup", "stdout", "stderr"):
                pairs[name] = os.pipe2(os.O_CLOEXEC)
            parent_fd = os.pidfd_open(os.getpid(), 0)
            os.unshare(os.CLONE_NEWPID)
            self.pid = os.fork()
            if self.pid == 0:
                for read_fd, _ in pairs.values():
                    os.close(read_fd)
                self.child(command, parent_fd, pairs["setup"][1], pairs["stdout"][1], pairs["stderr"][1])
                os._exit(125)
            self.receipt["init_host_pid"] = self.pid
            # 单线程且 SIGCHLD 未自动回收；未 wait 的直系 child 不存在 PID 复用窗口。
            self.pidfd = os.pidfd_open(self.pid, 0)
            self.receipt["init_pidfd_acquired"] = True
        finally:
            original = sys.exception()
            if parent_fd is not None:
                try:
                    os.close(parent_fd)
                except BaseException as error:
                    self.failure("parent_pidfd_close", original if original is not None else error)
                    if original is not None:
                        self.failure("parent_pidfd_close_secondary", error)
            for name, (read_fd, write_fd) in pairs.items():
                try:
                    os.close(write_fd)
                    if self.pid is None:
                        os.close(read_fd)
                    else:
                        self.streams[read_fd] = name
                        os.set_blocking(read_fd, False)
                except BaseException as error:
                    self.failure("spawn_pipe_finalization", original if original is not None else error)
                    if original is not None:
                        self.failure("spawn_pipe_secondary", error)

    def terminate(self):
        if self.pid is None or self.reaped:
            return
        try:
            if self.pidfd is not None:
                signal.pidfd_send_signal(self.pidfd, signal.SIGKILL)
            else:
                # 仅 pidfd_open 本身失败的未回收直系 child；不按枚举 PID 清理。
                os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except BaseException as error:
            self.failure("init_signal", error)

    def wait_once(self):
        if self.reaped or self.pid is None:
            return
        if self.pidfd is not None:
            result = os.waitid(os.P_PIDFD, self.pidfd, os.WEXITED | os.WNOHANG)
        else:
            result = os.waitid(os.P_PID, self.pid, os.WEXITED | os.WNOHANG)
        if result is not None:
            self.reaped = True
            self.receipt.update(init_wait_code=result.si_code, init_wait_status=result.si_status,
                                init_reaped=True, namespace_descendants_gone=True)

    def drain(self, selector):
        for key, _ in selector.select(0.05):
            fd, name = key.fd, key.data
            try:
                block = os.read(fd, 65536)
            except BlockingIOError:
                continue
            except OSError as error:
                self.failure(f"{name}_read", error)
                selector.unregister(fd)
                self.streams.pop(fd)
                try:
                    os.close(fd)
                except OSError as secondary:
                    self.failure(f"{name}_close", secondary)
                continue
            if not block:
                selector.unregister(fd)
                os.close(fd)
                self.streams.pop(fd)
                continue
            if name == "setup":
                if len(self.setup) + len(block) > 8192:
                    raise OverflowError("setup witness exceeded 8192 bytes")
                self.setup.extend(block)
            else:
                record = self.logs[name]
                room = (64 << 20) - record[1]
                if room > 0:
                    record[0].write(block[:room])
                    record[1] += min(room, len(block))
                if self.args.self_test and name == "stdout" and len(self.witness) < 8192:
                    self.witness.extend(block[:8192 - len(self.witness)])
                if len(block) > room:
                    raise OverflowError(f"{name} exceeded independent 64 MiB log cap")

    def monitor(self, deadline):
        with selectors.DefaultSelector() as selector:
            for fd, name in self.streams.items():
                selector.register(fd, selectors.EVENT_READ, name)
            cleanup_started = None
            while not self.reaped or self.streams:
                try:
                    if self.primary is None and time.monotonic() >= deadline:
                        raise TimeoutError("original namespace qualification deadline elapsed")
                    if self.primary is not None:
                        if cleanup_started is None:
                            cleanup_started = time.monotonic()
                        self.terminate()
                        if time.monotonic() - cleanup_started >= 240 and not self.receipt.get("cleanup_pending"):
                            self.receipt["cleanup_pending"] = True
                            self.save()
                            print("namespace cleanup pending; original owner retained until actual wait", file=sys.stderr)
                    self.wait_once()
                except BaseException as error:
                    self.failure("monitor", error)
                    self.terminate()
                    try:
                        time.sleep(0.05)
                    except BaseException as interrupted:
                        self.failure("cleanup_interrupt", interrupted)
                # IO编排损坏不能留在无事件循环；外层finally独立等待原init并关闭余下FD。
                try:
                    self.drain(selector)
                except BaseException as error:
                    self.failure("monitor_io", error)
                    self.terminate()
                    raise

    def qualify(self):
        records = [json.loads(line) for line in self.setup.splitlines()]
        if len(records) != 1 or records[0].get("phase") != "ready":
            raise RuntimeError(f"namespace setup failed: {records!r}")
        self.receipt["runner_identity"] = records[0]
        if records[0]["pid_namespace"] == self.receipt["outer_pid_namespace"]:
            raise RuntimeError("PID namespace did not change")
        if self.args.self_test:
            markers = [json.loads(line) for line in self.witness.splitlines() if line.startswith(b'{"descendant_pid"')]
            if (not isinstance(self.primary, TimeoutError) or not self.reaped or not markers or
                    self.witness.count(b"live-descendant\n") < 2 or
                    not any(item["descendant_pid"] > 1 and item["sid"] == item["descendant_pid"] and
                            item["pgid"] == item["descendant_pid"] for item in markers)):
                raise RuntimeError("real live setsid descendant timeout witness missing")
            self.receipt["setsid_timeout_witness"] = markers[0]
            self.receipt["status"] = "containment_self_test_passed_not_atomic_qualification"
            return
        if self.primary is not None:
            return
        if self.receipt["init_wait_code"] != os.CLD_EXITED or self.receipt["init_wait_status"] != 0:
            raise RuntimeError(f"qualification init failed: {self.receipt['init_wait_status']}")
        path = self.output / "qualification/receipt.json"
        if path.stat().st_size > (1 << 20):
            raise ValueError("qualification receipt exceeds 1 MiB")
        raw = path.read_bytes()
        child = json.loads(raw)
        profile = Path(self.command(self.args, self.output)[1]).name
        expected = PROFILES[profile]
        if (child.get("status") != "component_tests_passed_awaiting_outer_cleanup" or
                type(child.get("executed_parent_cases")) is not int or child["executed_parent_cases"] != expected):
            raise ValueError(f"the unchanged {expected} native cases for {profile} did not pass")
        self.receipt.update(status="native_qualification_and_namespace_reap_passed",
                            qualification_profile=profile, executed_parent_cases=expected,
                            qualification_receipt_sha256=hashlib.sha256(raw).hexdigest())

    def cleanup_owner(self):
        """即使 selector 初始化/读取失败，也保留原 owner 等实际 wait；不靠日志通道收尾。"""
        started = time.monotonic()
        while self.pid is not None and not self.reaped:
            self.terminate()
            try:
                self.wait_once()
                if time.monotonic() - started >= 240 and not self.receipt.get("cleanup_pending"):
                    self.receipt["cleanup_pending"] = True
                    self.save()
                    print("namespace cleanup pending; owner retained", file=sys.stderr)
            except BaseException as error:
                self.failure("final_wait", error)
            if not self.reaped:
                try:
                    time.sleep(0.05)
                except BaseException as error:
                    self.failure("final_wait_interrupt", error)
        for fd in list(self.streams):
            try:
                os.close(fd)
            except OSError as error:
                self.failure("final_read_fd_close", error)
            self.streams.pop(fd)

    def run(self):
        command = self.validate()
        self.output.mkdir(parents=True, exist_ok=False)
        os.chown(self.output, self.args.runner_uid, self.args.runner_gid)
        os.chmod(self.output, 0o700)
        self.receipt.update(command=command, timeout_seconds=self.args.timeout_seconds,
                            script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
        started = time.monotonic()
        deadline = started + self.args.timeout_seconds
        try:
            for name in ("stdout", "stderr"):
                self.logs[name] = [(self.output / f"child.{name}").open("wb"), 0]
            if not self.args.self_test and Path(command[1]).name == "qualify-linux-memfd-execution.py":
                factory = runpy.run_path(str(Path(__file__).with_name("native_memfd_profile.py")))["NativeMemfdProfile"]
                self.memfd_profile = factory(self, deadline)
                self.memfd_profile.load()
            signal.signal(signal.SIGCHLD, signal.SIG_DFL)
            self.spawn(command)
            self.monitor(deadline)
            self.qualify()
        except BaseException as error:
            self.failure("run", error)
        finally:
            while True:
                try:
                    self.cleanup_owner()
                    break
                except BaseException as error:
                    self.failure("owner_finalization", error)
                    if self.pid is None or self.reaped:
                        break
            for name, (stream, size) in self.logs.items():
                try:
                    stream.close()
                    self.receipt[f"{name}_bytes"] = size
                    self.receipt[f"{name}_sha256"] = hashlib.sha256((self.output / f"child.{name}").read_bytes()).hexdigest()
                except BaseException as error:
                    self.failure(f"{name}_finalization", error)
            if self.pidfd is not None and self.reaped:
                try:
                    os.close(self.pidfd)
                except BaseException as error:
                    self.failure("init_pidfd_close", error)
            if self.memfd_profile is not None:
                self.memfd_profile.finish()
            self.receipt["elapsed_seconds"] = time.monotonic() - started
            expected = self.args.self_test and self.receipt["status"] == "containment_self_test_passed_not_atomic_qualification"
            if self.primary is not None and (not expected or self.receipt["secondary_errors"]):
                self.receipt["status"] = "failed"
            self.save()
        expected = (self.args.self_test and self.receipt["status"] == "containment_self_test_passed_not_atomic_qualification"
                    and not self.receipt["secondary_errors"])
        if self.primary is not None and not expected:
            raise self.primary
        return 0

def descendant_fixture():
    """仅自测：真实后代 setsid 后持续工作，必须由 namespace init 退出收场。"""
    child = os.fork()
    if child == 0:
        os.setsid()
        print(json.dumps({"descendant_pid": os.getpid(), "sid": os.getsid(0), "pgid": os.getpgrp()}), flush=True)
        while True:
            print("live-descendant", flush=True)
            time.sleep(0.05)
    while True:
        time.sleep(1)


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--descendant-fixture":
        if sys.platform != "linux" or os.getpid() != 1:
            raise RuntimeError("fixture requires the real inner namespace init")
        descendant_fixture()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--runner-uid", type=int, required=True)
    parser.add_argument("--runner-gid", type=int, required=True)
    parser.add_argument("--runner-groups", type=lambda value: [int(item) for item in value.split(",")], required=True)
    parser.add_argument("--timeout-seconds", type=float, default=1800)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.self_test:
        if args.command:
            parser.error("self-test has a fixed real fixture, not a supplied command")
        args.command = [sys.executable, str(Path(__file__).resolve()), "--descendant-fixture", "{qualification_output}"]
        # token用于统一argv校验；fixture无磁盘工作，不创建qualification目录。
    return NativeNamespaceSupervisor(args).run()


if __name__ == "__main__":
    sys.exit(main())
