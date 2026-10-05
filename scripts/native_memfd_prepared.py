"""普通 runner 的有限准备/同 PID exact exec/清理；从不承担特权策略写入。"""
import json
import os
from pathlib import Path
import runpy
import shutil
import time


PLAN = runpy.run_path(str(Path(__file__).with_name("native_memfd_policy_plan.py")))["NativeMemfdPolicyPlan"]


class NativeMemfdPrepared:
    """来源：physical-scan-process 的2+1+3流程；产物保留不等于全案验收。"""

    def __init__(self, atomic, support, run_cases):
        self.atomic, self.support, self.run_cases = atomic, support, run_cases

    def complete(self, checkout, output, receipt, environment, manifest):
        """参数：已核来源与 inventory；返回：只完成普通三案的有界准备材料。"""
        command = ["cargo", "test", "--offline", "--locked", "-p", "diskgraph-engine", "--lib",
                   "--no-run", "--message-format=json"]
        log = self.atomic.run_step(receipt, output, "prepared-libtest", command, checkout,
                                  environment, self.support)
        executables = []
        for line in log.read_text().splitlines():
            if not line.startswith("{"):
                continue
            item = json.loads(line)
            if (item.get("reason") == "compiler-artifact" and item.get("profile", {}).get("test") is True
                    and item.get("target", {}).get("name") == "diskgraph_engine"
                    and item.get("target", {}).get("kind") == ["lib"] and item.get("executable")):
                executables.append(Path(item["executable"]))
        if len(executables) != 1:
            raise ValueError("exact original engine libtest artifact not identified")
        plan = PLAN.record(output, executables[0], receipt["compiled_inventory"])
        self.run_cases(checkout, output, receipt, environment, self.support,
                       manifest["test_groups"][:2], expected=3)
        (output / PLAN.FILE).write_text(json.dumps(plan, indent=2) + "\n")
        receipt.update(status="prepared_policy_cases_pending", executed_parent_cases=3,
                       prepared_record=PLAN.FILE,
                       prepared_record_sha256=PLAN.digest(output / PLAN.FILE))

    @staticmethod
    def credentials(output):
        """参数：固定原输出；返回：实际原 runner/NNP/caps0/PID1 见证，标量不代替查询。"""
        if os.geteuid() == 0 or os.getpid() != 1 or os.readlink("/proc/self") != "1":
            raise PermissionError("fixed prepared mode requires actual non-root namespace PID1")
        namespace = os.readlink("/proc/self/ns/pid")
        if (namespace != os.environ.get("DG_NATIVE_PID_NAMESPACE") or
                os.environ.get("DG_NATIVE_NAMESPACE_SUPERVISED") != "1" or
                output != Path(os.environ["DG_MEMFD_PREPARED_OUTPUT"]).resolve()):
            raise ValueError("prepared mode namespace/output material does not match actual execution")
        uid, gid = int(os.environ["DG_MEMFD_RUNNER_UID"]), int(os.environ["DG_MEMFD_RUNNER_GID"])
        groups = sorted(int(value) for value in os.environ["DG_MEMFD_RUNNER_GROUPS"].split(","))
        if (uid <= 0 or os.getresuid() != (uid,) * 3 or os.getresgid() != (gid,) * 3
                or sorted(os.getgroups()) != groups):
            raise PermissionError("actual prepared runner credentials mismatch")
        with Path("/proc/self/status").open("r") as stream:
            raw = stream.read((16 << 10) + 1)
        if len(raw) > (16 << 10):
            raise ValueError("bounded actual credential status exceeded")
        status = dict(line.split(":", 1) for line in raw.splitlines() if ":" in line)
        if (status["NoNewPrivs"].strip() != "1" or any(int(status[key].strip(), 16)
                for key in ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"))):
            raise PermissionError("actual permanent cap0/NNP1 qualification missing")
        return namespace

    @classmethod
    def case(cls, output, scope):
        """固定普通模式：核资格后同 PID exec 原 libtest，仅接受静态0/1/2映射。"""
        if type(scope) is not int or scope not in (0, 1, 2):
            raise ValueError("unknown fixed policy case")
        cls.credentials(output)
        end = int(os.environ["DG_MEMFD_DEADLINE_NS"])
        if time.monotonic_ns() >= end:
            raise TimeoutError("original policy case deadline elapsed before exact test exec")
        with Path("/proc/sys/vm/memfd_noexec").open("r") as stream:
            policy = stream.read(32)
        if policy.strip() != str(scope) or len(policy) >= 32:
            raise RuntimeError("actual namespace policy does not match fixed exact case")
        value = PLAN.verify_runner_material(output)
        binary = PLAN.path(output, value["binary"]["path"])
        environment = os.environ.copy()
        environment.update(DG_MEMFD_POLICY_INNER=str(scope), DG_MEMFD_PROC="/proc",
                           DG_LINUX_ATOMIC_LAUNCH_FIXTURE=str(PLAN.path(output, "fixtures/image-1")),
                           DG_LINUX_ATOMIC_LAUNCH_REPLACEMENT=str(PLAN.path(output, "fixtures/image-2")))
        if time.monotonic_ns() >= end:
            raise TimeoutError("original policy case deadline elapsed after prepared artifact checks")
        os.execve(binary, [str(binary), "--exact", PLAN.NAMES[scope], "--nocapture", "--test-threads=1"], environment)

    @classmethod
    def cleanup(cls, output):
        """普通 runner 仅回收固定准备目录；执行失败不冒充清理或原 pidfd wait 成功。"""
        cls.credentials(output)
        if time.monotonic_ns() >= int(os.environ["DG_MEMFD_CLEANUP_DEADLINE_NS"]):
            raise TimeoutError("original overall deadline expired before ordinary material cleanup")
        path = output / "prepared-checkout"
        if path.is_symlink() or (path.exists() and path != path.resolve(strict=True)):
            raise ValueError("prepared cleanup refuses aliased directory")
        if path.exists():
            shutil.rmtree(path)
        print("MEMFD_PREPARED_CLEANUP_COMPLETE", flush=True)
