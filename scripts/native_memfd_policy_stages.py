"""固定0/1/2的独占 PID1 QA；root 只写固定策略，runner 才消费准备代码。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import time


PLAN = runpy.run_path(str(Path(__file__).with_name("native_memfd_policy_plan.py")))["NativeMemfdPolicyPlan"]


ARTIFACTS = runpy.run_path(str(Path(__file__).with_name("native_namespace_artifacts.py")))["NativeNamespaceArtifacts"]


class NativeMemfdPolicyStages:
    """来源：physical-scan-process 两阶段流程；每案持唯一原 init/pidfd 至实际 wait。"""

    def __init__(self, supervisor, deadline, command, finalize):
        self.supervisor, self.deadline, self.command, self.finalize = supervisor, deadline, command, finalize
        self.prepared = supervisor.output / "runner/qualification"

    @staticmethod
    def qualify_case(child, scope, raw):
        """参数：已实际 wait 的原 owner 与双日志；返回：精确原案的单次完成记录。"""
        records = [json.loads(line) for line in child.setup.splitlines()]
        if len(records) != 1 or records[0].get("phase") != "ready":
            raise RuntimeError(f"fixed policy namespace setup failed: {records!r}")
        identity = records[0]
        child.receipt["runner_identity"] = identity
        if (not child.reaped or child.streams or child.receipt.get("init_wait_code") != os.CLD_EXITED
                or child.receipt.get("init_wait_status") != 0):
            raise RuntimeError("fixed policy case original init wait/capture did not complete successfully")
        policy = identity.get("memfd_policy_setup", {})
        if (identity.get("pid") != 1 or identity.get("pid_namespace") == child.receipt["outer_pid_namespace"]
                or identity.get("uid") != [child.args.runner_uid] * 3
                or identity.get("gid") != [child.args.runner_gid] * 3
                or identity.get("groups") != sorted(child.args.runner_groups)
                or identity.get("no_new_privs") != "1"
                or set(identity.get("capabilities", {})) != {"CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"}
                or any(int(value, 16) for value in identity["capabilities"].values())):
            raise PermissionError("actual fixed policy non-root/NNP/cap0/PID1 witness missing")
        if (policy.get("pid") != 1 or policy.get("proc_self") != "1" or policy.get("setup_uid") != 0
                or policy.get("pid_namespace") != identity["pid_namespace"]
                or policy.get("fixed_path") != "/proc/sys/vm/memfd_noexec"
                or policy.get("actual_policy") != scope or type(policy.get("inherited_policy")) is not int
                or not 0 <= policy["inherited_policy"] <= scope):
            raise PermissionError("actual fixed GLOBAL UID0 write/read/inherited-floor witness missing")
        summaries = re.findall(rb"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", raw)
        if (len(summaries) != 1 or tuple(map(int, summaries[0])) != (1, 0, 0)
                or PLAN.NAMES[scope].encode() not in raw
                or f"MEMFD_POLICY_CASE_COMPLETE scope={scope}".encode() not in raw):
            raise ValueError("original exact policy body did not actually pass one case without skips")
        child.receipt.update(status="fixed_policy_case_and_namespace_reap_passed", executed_parent_cases=1,
                             exact_case=PLAN.NAMES[scope], policy_scope=scope)

    def invoke(self, name, scope=None):
        """只产生固定内部 argv；所有 runner 可写程序在 child 永久降权后才 exec。"""
        original = self.supervisor
        args = argparse.Namespace(**vars(original.args))
        args.output_dir = original.output / name
        child = type(original)(args)
        child.prepared_output = self.prepared
        child.policy_scope = scope
        child.prepared_cleanup_deadline_ns = int(self.deadline * 1_000_000_000) if scope is None else None
        started = time.monotonic()
        end = min(self.deadline, started + 20) if scope is not None else self.deadline
        child.policy_deadline_ns = int(end * 1_000_000_000) if scope is not None else None
        mode = ["--cleanup-prepared"] if scope is None else ["--policy-case", str(scope)]
        command = self.command[:2] + mode + ["--output-dir", str(self.prepared)]
        child.receipt.update(command=command, policy_scope=scope, original_deadline_ns=child.policy_deadline_ns)
        try:
            if time.monotonic() >= end:
                raise TimeoutError("original overall/case deadline expired before fixed child birth")
            ARTIFACTS.initialize(child, original.artifact_dir_fd, name)
            for channel in ("stdout", "stderr"):
                child.logs[channel] = [ARTIFACTS.open_log(child, channel), 0]
            child.spawn(command)
            child.monitor(end)
            if child.primary is None:
                # monitor 已完成 capture；文件缓冲必须在实读日志前实际 flush。
                for stream, _ in child.logs.values():
                    stream.flush()
                raw = b"\n".join(ARTIFACTS.capture(*child.logs[channel])
                                  for channel in ("stdout", "stderr"))
                if scope is not None:
                    self.qualify_case(child, scope, raw)
                elif (not child.reaped or child.streams or child.receipt.get("init_wait_code") != os.CLD_EXITED
                      or child.receipt.get("init_wait_status") != 0
                      or b"MEMFD_PREPARED_CLEANUP_COMPLETE" not in raw):
                    raise RuntimeError("ordinary runner prepared-material cleanup did not complete")
                else:
                    child.receipt["status"] = "ordinary_prepared_material_cleanup_and_reap_passed"
        except BaseException as error:
            child.failure("fixed_policy_stage", error)
        finally:
            self.finalize(child)
            child.receipt["elapsed_seconds"] = time.monotonic() - started
            if child.primary is not None:
                child.receipt["status"] = "failed"
            child.save()
            try:
                ARTIFACTS.close(child)
            except BaseException as error:
                child.failure("record_directory_close", error)
            original.receipt.setdefault("policy_stages", []).append({**child.receipt, "scope": scope,
                "namespace_scope": child.receipt["scope"], "receipt":
                str(child.output.relative_to(original.output) / "namespace-receipt.json")})
            if child.primary is not None:
                original.failure("policy_material_cleanup" if scope is None else f"policy_{scope}", child.primary)

    def run(self):
        """普通三案与三份实际独占 wait 均成功，且普通清理完成后才能累计六案。"""
        supervisor = self.supervisor
        if not supervisor.reaped or supervisor.streams or supervisor.receipt["executed_parent_cases"] != 3:
            raise RuntimeError("ordinary prepare owner must finish before fixed policy stages")
        try:
            record = PLAN.read(self.prepared)
            if PLAN.digest(self.prepared / PLAN.FILE) != supervisor.receipt["prepared_record_sha256"]:
                raise ValueError("prepared record changed after original prepare receipt")
            if record["inventory"][-3:] != list(PLAN.NAMES):
                raise ValueError("fixed original policy inventory changed")
            for scope in (0, 1, 2):
                self.invoke(f"policy-{scope}", scope)
        except BaseException as error:
            supervisor.failure("policy_plan_or_stage", error)
        finally:
            # 所有 case 的原 wait/capture 已在 invoke.finally 内完成，才以普通 UID 清理材料。
            supervisor.receipt["prepared_cleanup_attempted"] = True
            self.invoke("prepared-cleanup")
        if supervisor.primary is not None:
            raise supervisor.primary
        stages = supervisor.receipt["policy_stages"]
        if (len(stages) != 4 or any(not stage.get("init_reaped") or
                                  not stage.get("namespace_descendants_gone") for stage in stages)):
            raise RuntimeError("fixed policy stage owner/capture closure incomplete")
        supervisor.receipt.update(status="native_qualification_and_namespace_reap_passed",
                                 qualification_profile="qualify-linux-memfd-execution.py", executed_parent_cases=6,
                                 prepared_record_sha256=PLAN.digest(self.prepared / PLAN.FILE))
