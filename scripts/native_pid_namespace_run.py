"""唯一外层 owner 的执行与回收；固定两阶段不会另建隐藏进程所有者。"""
import hashlib
import os
from pathlib import Path
import runpy
import signal
import time


ARTIFACTS = runpy.run_path(str(Path(__file__).with_name("native_namespace_artifacts.py")))["NativeNamespaceArtifacts"]


PROFILE = runpy.run_path(str(Path(__file__).with_name("native_memfd_profile.py")))["NativeMemfdProfile"]
STAGES = runpy.run_path(str(Path(__file__).with_name("native_memfd_policy_stages.py")))["NativeMemfdPolicyStages"]


class NativeNamespaceRun:
    """来源：原 supervisor.run 的实际 wait/日志/profile 收尾，保留首错原对象。"""

    def __init__(self, supervisor, source):
        self.supervisor, self.source = supervisor, source

    @staticmethod
    def finalize(supervisor):
        """参数：持原 init 的 supervisor；返回：只有原 wait 消费后才关闭 pidfd。"""
        while True:
            try:
                supervisor.cleanup_owner()
                break
            except BaseException as error:
                supervisor.failure("owner_finalization", error)
                if supervisor.pid is None or supervisor.reaped:
                    break
        for name, (stream, size) in supervisor.logs.items():
            if stream.closed:
                continue  # 两阶段会再次收尾；已记录原摘要不得被空内容覆盖。
            try:
                raw = ARTIFACTS.capture(stream, size)
                supervisor.receipt[f"{name}_bytes"] = size
                supervisor.receipt[f"{name}_sha256"] = hashlib.sha256(raw).hexdigest()
            except BaseException as error:
                supervisor.failure(f"{name}_finalization", error)
            finally:
                try:
                    stream.close()
                except BaseException as error:
                    supervisor.failure(f"{name}_close", error)
        if supervisor.pidfd is not None and supervisor.reaped:
            try:
                os.close(supervisor.pidfd)
                supervisor.pidfd = None
            except BaseException as error:
                supervisor.failure("init_pidfd_close", error)

    def run(self):
        """参数：已绑定 wrapper；返回：原资格结果，任何清理次错均阻止成功。"""
        supervisor = self.supervisor
        command = supervisor.validate()
        ARTIFACTS.initialize(supervisor)
        supervisor.receipt.update(command=command, timeout_seconds=supervisor.args.timeout_seconds,
                                  script_sha256=hashlib.sha256(self.source.read_bytes()).hexdigest())
        started = time.monotonic()
        deadline = started + supervisor.args.timeout_seconds
        try:
            for name in ("stdout", "stderr"):
                supervisor.logs[name] = [ARTIFACTS.open_log(supervisor, name), 0]
            if not supervisor.args.self_test and Path(command[1]).name == "qualify-linux-memfd-execution.py":
                supervisor.memfd_profile = PROFILE(supervisor, deadline)
                supervisor.memfd_profile.load()
            signal.signal(signal.SIGCHLD, signal.SIG_DFL)
            supervisor.spawn(command)
            supervisor.monitor(deadline)
            supervisor.qualify()
            if supervisor.receipt["status"] == "prepared_policy_cases_pending":
                self.finalize(supervisor)
                if supervisor.primary is not None:
                    raise supervisor.primary
                STAGES(supervisor, deadline, command, self.finalize).run()
        except BaseException as error:
            supervisor.failure("run", error)
        finally:
            self.finalize(supervisor)
            if (Path(command[1]).name == "qualify-linux-memfd-execution.py"
                    and (supervisor.output / "runner/qualification/prepared-checkout").exists()
                    and not supervisor.receipt.get("prepared_cleanup_attempted")):
                # 准备中途失败也不能由 root 删除 runner 材料；原 owner 回收后走同一个固定普通入口。
                supervisor.receipt["prepared_cleanup_attempted"] = True
                try:
                    STAGES(supervisor, deadline, command, self.finalize).invoke("prepared-cleanup")
                except BaseException as error:
                    supervisor.failure("prepared_material_finalization", error)
            if supervisor.memfd_profile is not None:
                supervisor.memfd_profile.finish()
            supervisor.receipt["elapsed_seconds"] = time.monotonic() - started
            expected = (supervisor.args.self_test and supervisor.receipt["status"] ==
                        "containment_self_test_passed_not_atomic_qualification")
            if supervisor.primary is not None and (not expected or supervisor.receipt["secondary_errors"]):
                supervisor.receipt["status"] = "failed"
            supervisor.save()
            try:
                ARTIFACTS.close(supervisor)
            except BaseException as error:
                supervisor.failure("record_directory_close", error)
        expected = (supervisor.args.self_test and supervisor.receipt["status"] ==
                    "containment_self_test_passed_not_atomic_qualification" and
                    not supervisor.receipt["secondary_errors"])
        if supervisor.primary is not None and not expected:
            raise supervisor.primary
        return 0
