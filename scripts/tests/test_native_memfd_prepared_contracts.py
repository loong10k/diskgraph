"""固定两阶段材料与外层验收的纯合同；不把模型 PID/cap/wait 当 Linux 运行证明。"""
import argparse
import json
import os
from pathlib import Path
import runpy
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SCRIPTS = Path(__file__).resolve().parents[1]
PLAN = runpy.run_path(str(SCRIPTS / "native_memfd_policy_plan.py"))["NativeMemfdPolicyPlan"]
STAGES = runpy.run_path(str(SCRIPTS / "native_memfd_policy_stages.py"))["NativeMemfdPolicyStages"]
PREPARED = runpy.run_path(str(SCRIPTS / "native_memfd_prepared.py"))["NativeMemfdPrepared"]
SUPERVISOR = runpy.run_path(str(SCRIPTS / "run-native-pid-namespace.py"))["NativeNamespaceSupervisor"]


class NativeMemfdPreparedContracts(unittest.TestCase):
    """只测装配与拒绝条件，原生 GLOBAL UID0/privateproc/原 wait 另由 CI 核验。"""

    def arguments(self, output):
        return argparse.Namespace(output_dir=output, runner_uid=1001, runner_gid=1001,
                                  runner_groups=[1001], timeout_seconds=20, self_test=False,
                                  command=[os.sys.executable, str(SCRIPTS / "qualify-linux-memfd-execution.py"),
                                           "--output-dir", "{qualification_output}"])

    def qualified_model(self):
        identity = {"phase": "ready", "pid": 1, "pid_namespace": "pid:[modeled-private]",
                    "uid": [1001] * 3, "gid": [1001] * 3, "groups": [1001], "no_new_privs": "1",
                    "capabilities": {key: "0000000000000000" for key in
                                     ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")},
                    "memfd_policy_setup": {"pid": 1, "proc_self": "1", "setup_uid": 0,
                                           "pid_namespace": "pid:[modeled-private]",
                                           "fixed_path": "/proc/sys/vm/memfd_noexec",
                                           "actual_policy": 0, "inherited_policy": 0}}
        child = SimpleNamespace(args=self.arguments(Path("/tmp/no-actual-child")), reaped=True, streams={},
                                setup=json.dumps(identity).encode(), receipt={"outer_pid_namespace": "pid:[model-outer]",
                                "init_wait_code": 1, "init_wait_status": 0})
        raw = (PLAN.NAMES[0] + "\nMEMFD_POLICY_CASE_COMPLETE scope=0\n"
               "test result: ok. 1 passed; 0 failed; 0 ignored;\n").encode()
        return child, identity, raw

    def test_exact_single_case_model_and_original_reap_are_both_required(self):
        child, _, raw = self.qualified_model()
        with patch.object(os, "CLD_EXITED", 1, create=True):
            STAGES.qualify_case(child, 0, raw)
            self.assertEqual(child.receipt["executed_parent_cases"], 1)
            child.reaped = False
            with self.assertRaisesRegex(RuntimeError, "original init wait"):
                STAGES.qualify_case(child, 0, raw)

    def test_environment_policy_scalar_cannot_replace_actual_root_readback(self):
        child, identity, raw = self.qualified_model()
        identity["memfd_policy_setup"]["actual_policy"] = 1
        child.setup = json.dumps(identity).encode()
        with patch.object(os, "CLD_EXITED", 1, create=True), self.assertRaises(PermissionError):
            STAGES.qualify_case(child, 0, raw)

    def test_caps_and_original_credentials_remain_mandatory_after_setup(self):
        for field in ("capabilities", "uid", "no_new_privs"):
            child, identity, raw = self.qualified_model()
            if field == "capabilities":
                identity[field]["CapEff"] = "0000000000200000"
            elif field == "uid":
                identity[field] = [0] * 3
            else:
                identity[field] = "0"
            child.setup = json.dumps(identity).encode()
            with self.subTest(field=field), patch.object(os, "CLD_EXITED", 1, create=True), \
                    self.assertRaises(PermissionError):
                STAGES.qualify_case(child, 0, raw)

    def test_skipped_or_extra_parent_cases_do_not_qualify(self):
        child, _, raw = self.qualified_model()
        with patch.object(os, "CLD_EXITED", 1, create=True):
            for summary in (b"0 passed; 0 failed; 1 ignored;", b"2 passed; 0 failed; 0 ignored;"):
                changed = raw.replace(b"1 passed; 0 failed; 0 ignored;", summary)
                with self.subTest(summary=summary), self.assertRaises(ValueError):
                    STAGES.qualify_case(child, 0, changed)

    def test_root_cannot_load_prepared_executable_even_with_valid_record(self):
        with patch.object(os, "geteuid", return_value=0), \
                self.assertRaisesRegex(PermissionError, "never execute as root"):
            PLAN.verify_runner_material(Path("/tmp/not-opened"))
        with patch.object(os, "geteuid", return_value=0), patch.object(os, "execve") as execute, \
                self.assertRaises(PermissionError):
            PREPARED.case(Path("/tmp/not-opened"), 0)
        execute.assert_not_called()

    def test_closed_prepared_record_rejects_free_artifact_and_unknown_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary).resolve()
            base = {"schema_version": 1, "checkout": "prepared-checkout", "inventory":
                    ["modeled-e1", "modeled-e2", "modeled-d1", *PLAN.NAMES], "binary":
                    {"path": "prepared-checkout/target-memfd-qualification/debug/deps/diskgraph_engine-abcd",
                     "sha256": "a" * 64, "bytes": 1}, "fixtures":
                    [{"path": f"fixtures/image-{number}", "sha256": "b" * 64, "bytes": 1} for number in (1, 2)]}
            for change in (lambda value: value.update(arbitrary_root_argv=["/bin/sh"]),
                           lambda value: value["binary"].update(path="../../free-program")):
                value = json.loads(json.dumps(base))
                change(value)
                (output / PLAN.FILE).write_text(json.dumps(value))
                with self.assertRaises(ValueError):
                    PLAN.read(output)

    def test_stage_cleanup_error_retains_first_original_error_object(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary).resolve()
            outer = SUPERVISOR(self.arguments(output))
            original = TimeoutError("fixed original pre-birth failure")
            secondary = OSError(5, "modeled finalization failure")

            def finalize(child):
                child.failure("actual-finalize-model", secondary)

            stages = STAGES(outer, time.monotonic() + 60, self.arguments(output).command[:2], finalize)
            with patch.object(os, "chown"), patch.object(SUPERVISOR, "spawn", side_effect=original):
                stages.invoke("policy-0", 0)
            self.assertIs(outer.primary, original)
            record = outer.receipt["policy_stages"][0]
            self.assertEqual(record["status"], "failed")
            self.assertEqual(record["secondary_errors"][0]["errno"], 5)
            self.assertNotIn("init_reaped", record)


if __name__ == "__main__":
    unittest.main()
