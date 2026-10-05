"""Windows实际child清理失败的冻结RED/GREEN验收；不声称完整扫描资格。"""
import argparse
import json
import os
import platform
import subprocess
from pathlib import Path

import qualify_macos_installed_worker as shared

CANDIDATE = Path("crates/diskgraph-engine/integration_candidates/windows_cleanup")


REQUIRED_CASES = tuple("native_child::windows::windows_cleanup_recovery_tests::" + name for name in (
    "cleanup_wait_failure_retains_original_owner_until_actual_retry",
    "cleanup_query_failure_retains_original_job_until_actual_retry",
    "recovery_slot_survives_query_failure_until_original_job_is_observed_empty",
))


IO_PREREQUISITE_CASES = tuple("native_child::windows::windows_io_transfer_tests::" + name for name in (
    "pending_read_owner_moves_threads_with_stable_storage_and_cancellation",
    "pending_read_query_failure_keeps_kernel_memory_until_other_thread_cleanup",
    "pending_write_owner_moves_threads_with_stable_storage_and_cancellation",
    "pending_read_drop_on_receiving_thread_waits_for_real_completion",
))


def check_prerequisite_cases(cases):
    if len(cases) != len(IO_PREREQUISITE_CASES) or set(cases) != set(IO_PREREQUISITE_CASES):
        raise ValueError("fixed pending IO prerequisite inventory differs")


BIRTH_CASES = tuple("native_child::windows::windows_birth_recovery_tests::" + name for name in (
    "postbirth_panic_keeps_external_owner_through_failed_wait_and_recovery",
    "postbirth_checkpoint_error_keeps_external_owner_through_failed_wait",
    "postbirth_checkpoint_error_keeps_external_owner_through_failed_job_query",
))


def check_birth_cases(cases):
    if len(cases) != len(BIRTH_CASES) or set(cases) != set(BIRTH_CASES):
        raise ValueError("fixed birth ownership acceptance inventory differs")


REGRESSION_CASES = tuple("native_child::windows::windows_normal_exit_tests::" + name for name in (
    "normal_exit_waits_for_stdio_closed_descendant_after_leader_has_exited",
    "normal_exit_does_not_accept_end_and_two_eofs_while_leader_remains_alive",
    "normal_exit_requires_control_closed_and_preserves_real_nonzero_leader_code",
    "normal_exit_checkpoint_retains_nonclone_error_without_killing_qualified_job",
)) + tuple("native_child::windows::windows_control_input_tests::" + name for name in (
    "original_null_spawn_has_four_checkpoints_and_no_control_input",
    "third_original_checkpoint_preserves_non_clone_primary_and_cleans_the_suspended_job",
    "original_post_create_failure_cleans_actual_job_with_external_witness_handles_held",
))


def check_regression_cases(cases):
    if len(cases) != len(REGRESSION_CASES) or set(cases) != set(REGRESSION_CASES):
        raise ValueError("fixed normal birth regression inventory differs")


def check_cases(cases):
    if len(cases) != len(REQUIRED_CASES) or set(cases) != set(REQUIRED_CASES):
        raise ValueError("fixed cleanup acceptance inventory differs")


def check_platform(system, github_actions, runner):
    if system != "Windows" or github_actions != "true" or runner != "github-hosted":
        raise RuntimeError("actual Windows hosted CI required")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    checkout = Path(__file__).resolve().parent.parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    receipt = {"schema_version": 1, "production_acceptance": False, "status": "started"}
    try:
        check_platform(platform.system(), os.environ.get("GITHUB_ACTIONS"), os.environ.get("RUNNER_ENVIRONMENT"))
        receipt["checkout_sha"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
        receipt["rustc"] = subprocess.check_output(["rustc", "--version"], text=True).strip()
        receipt["platform"] = platform.platform()
        receipt["candidate"] = manifest = shared.mount(checkout, CANDIDATE)
        cases = manifest["cleanup_cases"]
        check_cases(cases)
        prerequisites = manifest["io_prerequisite_cases"]
        check_prerequisite_cases(prerequisites)
        birth_cases = manifest["birth_cases"]
        check_birth_cases(birth_cases)
        regression_cases = manifest["regression_cases"]
        check_regression_cases(regression_cases)
        environment = os.environ.copy()
        shared.invoke(["cargo", "test", "--locked", "-p", "diskgraph-engine", "--lib", "--no-run", "--message-format=json"],
                      checkout, output, "build-cleanup-fixtures", environment)
        binaries = []
        for line in (output / "build-cleanup-fixtures.stdout").read_text().splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "diskgraph_engine" and record.get("executable"):
                    binaries.append(Path(record["executable"]))
        if len(binaries) != 1:
            raise RuntimeError("expected one actual Engine test executable")
        binary = binaries[0]
        receipt["fixture_sha256"] = shared.digest(binary)
        results = []
        # 每案独立进程，真实RED不阻止其余案取证；失败仍原样保留并使总门禁失败。
        receipt["cases"] = results
        for case in list(prerequisites) + list(cases) + list(birth_cases) + list(regression_cases):
            name = case.rsplit("::", 1)[-1]
            try:
                shared.invoke([str(binary), case, "--exact", "--nocapture", "--test-threads=1"],
                              checkout, output, name, environment, timeout=120)
                passed = "test result: ok. 1 passed; 0 failed;" in (output / (name + ".stdout")).read_text()
            except RuntimeError:
                passed = False
            results.append({"case": case, "passed": passed, "phase": "io_prerequisite" if case in prerequisites else "birth" if case in birth_cases else "regression" if case in regression_cases else "cleanup"})
            if case == prerequisites[-1] and not all(result["passed"] for result in results):
                raise RuntimeError("pending IO prerequisites failed; cleanup qualification not started")
        receipt["cases"] = results
        for name, expected in manifest["sources"].items():
            if shared.digest(checkout / name) != expected:
                raise RuntimeError("qualification modified candidate source")
        if shared.digest(binary) != receipt["fixture_sha256"]:
            raise RuntimeError("qualification binary identity changed")
        if not all(case["passed"] for case in results):
            raise RuntimeError("actual ownership cases failed; inspect preserved original logs")
        receipt["status"] = "passed"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
