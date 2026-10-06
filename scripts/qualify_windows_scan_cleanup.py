"""Windows实际child清理失败的冻结RED/GREEN验收；不声称完整扫描资格。"""
import argparse
import json
import os
import platform
import subprocess
from pathlib import Path

import qualify_macos_installed_worker as shared

BASELINE_CANDIDATE = Path("crates/diskgraph-engine/integration_candidates/windows_cleanup_baseline")
CANDIDATE = Path("crates/diskgraph-engine/integration_candidates/windows_cleanup")

DIRECTORY_ENUMERATION_CASES = tuple("live_evidence::windows_git_directory_cursor_tests::" + name for name in (
    "held_directory_names_ignore_foreign_path_argument",
    "held_directory_pages_preserve_raw_utf16_and_full_file_ids",
    "directory_record_decoder_rejects_invalid_lengths_offsets_and_components",
    "verified_cleanup_child_uses_original_parent_after_move_and_foreign_root_replacement",
    "verified_cleanup_child_rejects_same_name_foreign_replacement_and_type_mismatch",
    "verified_cleanup_child_refuses_real_junction_without_touching_external_target",
    "verified_cleanup_child_preserves_real_share_read_conflict_and_retries_same_id",
))


def check_directory_enumeration_cases(cases):
    if len(cases) != len(DIRECTORY_ENUMERATION_CASES) or set(cases) != set(DIRECTORY_ENUMERATION_CASES):
        raise ValueError("fixed held directory enumeration and codec inventory differs")


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


PROBE_CASES = tuple("native_child::windows::windows_probe_recovery_tests::" + name for name in (
    "managed_probe_cancel_wait_failure_retains_original_owner_and_capacity",
    "managed_probe_cancel_query_failure_retains_original_owner_and_capacity",
    "managed_probe_panic_wait_failure_retains_original_owner_and_capacity",
    "expired_managed_probe_transfers_original_owner_without_legacy_wait",
))


def check_probe_cases(cases):
    if len(cases) != len(PROBE_CASES) or set(cases) != set(PROBE_CASES):
        raise ValueError("fixed managed probe recovery inventory differs")


DIRECTORY_CASES = tuple("native_child::windows::windows_probe_directory_recovery_tests::" + name for name in (
    "real_git_private_directory_creation_and_completion_qualifies_resource_fixture",
    "private_git_complete_retains_original_directory_after_cancel_cleanup_failure",
    "private_git_drop_retains_original_directory_after_panic_cleanup_failure",
))


def check_directory_cases(cases):
    if tuple(cases) != DIRECTORY_CASES:
        raise ValueError("fixed private directory prerequisite and recovery order differs")


RESOURCE_CASES = tuple("live_evidence::probe_resource_pool_tests::" + name for name in (
    "real_directory_delete_failure_keeps_session_slot_until_native_release",
    "borrowed_directory_prevents_session_reuse_after_budget_drop",
))


def check_resource_cases(cases):
    if len(cases) != len(RESOURCE_CASES) or set(cases) != set(RESOURCE_CASES):
        raise ValueError("fixed resource slot native inventory differs")


RUNNER_CASES = tuple("runner::tests::" + name for name in (
    "stop_preserves_original_background_panic_payload",
    "managed_runner_rejects_recovery_from_another_pool_before_start",
    "busy_managed_runner_does_not_claim_or_fail_queued_job",
    "active_probe_session_does_not_turn_queued_job_into_capacity_failure",
            "recovered_runner_admission_does_not_remain_poisoned_after_panic",
))


def check_runner_cases(cases):
    if len(cases) != len(RUNNER_CASES) or set(cases) != set(RUNNER_CASES):
        raise ValueError("fixed runner lifecycle native inventory differs")


POLL_CLEANUP_CASES = tuple("native_child::windows::windows_poll_cleanup_tests::" + name for name in (
    "expired_read_cleanup_keeps_pending_storage_until_other_thread_completes",
    "expired_write_cleanup_keeps_pending_storage_until_other_thread_completes",
    "expired_child_cleanup_keeps_original_handles_until_native_complete",
    "poll_cleanup_wait_failure_retains_original_owner_until_actual_retry",
    "poll_cleanup_query_failure_retains_original_owner_until_actual_retry",
))


def check_poll_cleanup_cases(cases):
    if len(cases) != len(POLL_CLEANUP_CASES) or set(cases) != set(POLL_CLEANUP_CASES):
        raise ValueError("fixed finite cleanup native inventory differs")


REGISTRY_DEADLINE_CASES = tuple("native_child::windows::windows_registry_deadline_tests::" + name for name in (
    "expired_registry_drain_retains_original_owner_and_capacity_until_actual_retry",
    "registry_deadline_wait_error_preserves_original_owner_and_capacity",
    "registry_deadline_query_error_preserves_original_owner_and_capacity",
)) + ("scan_worker_registry::windows_registry_lock_tests::contended_registry_drain_returns_pending_without_taking_reserved_slot",)


def check_registry_deadline_cases(cases):
    if len(cases) != len(REGISTRY_DEADLINE_CASES) or set(cases) != set(REGISTRY_DEADLINE_CASES):
        raise ValueError("fixed registry deadline native inventory differs")


MISSING_NAME_CASES = tuple("live_evidence::git_private_directory_owner_tests::" + name for name in (
    "moved_original_directory_is_not_complete_when_original_path_is_missing",
    "foreign_replacement_is_retained_until_original_directory_returns",
))


def check_missing_name_cases(cases):
    if len(cases) != len(MISSING_NAME_CASES) or set(cases) != set(MISSING_NAME_CASES):
        raise ValueError("fixed missing-name recovery inventory differs")


ATOMIC_ROOT_CASES = tuple("live_evidence::windows_git_private_root_tests::" + name for name in (
    "atomic_root_create_refuses_existing_directory_without_adopting_it",
    "moved_root_delete_reopen_targets_original_object_not_foreign_replacement",
    "original_anchor_allows_real_managed_git_cwd_and_unmodified_directory_lease",
    "moved_root_reopen_ignores_junction_replacement_and_preserves_external_target",
    "original_share_read_lease_blocks_delete_reopen_until_released",
    "held_parent_lease_blocks_move_until_creation_phase_is_released",
))


def check_atomic_root_cases(cases):
    if len(cases) != len(ATOMIC_ROOT_CASES) or set(cases) != set(ATOMIC_ROOT_CASES):
        raise ValueError("fixed atomic root native inventory differs")


PREPARED_CONNECT_CASES = tuple("native_child::windows::windows_prepared_connect_tests::" + name for name in (
    "expired_connect_cleanup_retains_original_addresses_without_eof",
    "query_failure_retains_original_connect_for_actual_retry",
    "actual_connect_completion_allows_read_then_distinct_read_cancellation",
    "external_owner_survives_panic_after_actual_connect_submission",
    "pending_connect_moves_threads_without_replacing_original_storage",
))


def check_prepared_connect_cases(cases):
    if len(cases) != len(PREPARED_CONNECT_CASES) or set(cases) != set(PREPARED_CONNECT_CASES):
        raise ValueError("fixed actual pending Connect inventory differs")


PREPARED_JOB_CASES = tuple("native_child::windows::windows_child::windows_child_prepared_tests::" + name for name in (
    "prepared_pending_connect_retains_capacity_until_real_job_and_io_complete",
    "prepared_job_query_error_keeps_original_connect_owner_for_retry",
    "unknown_creation_phase_with_missing_leader_never_releases_original_job",
    "panic_after_prepared_connect_keeps_original_external_job_and_storage",
    "checked_admission_rejects_before_job_without_advancing_lifecycle",
    "checked_admission_rejects_prepared_pipe_without_advancing_lifecycle",
    "actual_pending_connect_wait_expires_on_original_admission_deadline",
)) + ("native_child::windows::windows_birth_recovery_tests::actual_create_process_failure_retains_external_job_until_observed_cleanup",)


def check_prepared_job_cases(cases):
    if len(cases) != len(PREPARED_JOB_CASES) or set(cases) != set(PREPARED_JOB_CASES):
        raise ValueError("fixed prepared Job and actual failure inventory differs")


def check_cases(cases):
    if len(cases) != len(REQUIRED_CASES) or set(cases) != set(REQUIRED_CASES):
        raise ValueError("fixed cleanup acceptance inventory differs")


FENCE_PRIORITY_CASES = tuple("process_fence_priority_tests::" + name for name in (
    "live_public_claim_and_original_authority_reach_fence_work",
    "original_expiry_remains_authority_denied_when_execution_deadline_already_elapsed",
    "current_owner_and_live_metadata_grant_still_gate_unexpired_work",
    "wrong_owner_is_rejected_before_unexpired_fence_work",
))

def check_fence_priority_cases(cases):
    if len(cases) != len(FENCE_PRIORITY_CASES) or set(cases) != set(FENCE_PRIORITY_CASES):
        raise ValueError("fixed request fence priority inventory differs")


def check_platform(system, github_actions, runner):
    if system != "Windows" or github_actions != "true" or runner != "github-hosted":
        raise RuntimeError("actual Windows hosted CI required")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--baseline", action="store_true", help="run the fixed pre-birth source comparison")
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
        from qualify_windows_prepared_birth_baseline import isolated_source
        with isolated_source(checkout, BASELINE_CANDIDATE if args.baseline else CANDIDATE) as (source, manifest):
            receipt["candidate"] = manifest
            receipt["workspace_commit"] = manifest["base_ref"]
            enumeration_cases = [] if args.baseline else manifest["directory_enumeration_cases"]
            if not args.baseline:
                check_directory_enumeration_cases(enumeration_cases)
            cases = manifest["cleanup_cases"]
            check_cases(cases)
            prerequisites = manifest["io_prerequisite_cases"]
            check_prerequisite_cases(prerequisites)
            birth_cases = manifest["birth_cases"]
            check_birth_cases(birth_cases)
            regression_cases = manifest["regression_cases"]
            check_regression_cases(regression_cases)
            probe_cases = [] if args.baseline else manifest["probe_cases"]
            if not args.baseline:
                check_probe_cases(probe_cases)
            directory_cases = [] if args.baseline else manifest["directory_cases"]
            if not args.baseline:
                check_directory_cases(directory_cases)
            resource_cases = [] if args.baseline else manifest["resource_cases"]
            if not args.baseline:
                check_resource_cases(resource_cases)
            runner_cases = [] if args.baseline else manifest["runner_cases"]
            if not args.baseline:
                check_runner_cases(runner_cases)
            poll_cases = [] if args.baseline else manifest["poll_cleanup_cases"]
            if not args.baseline:
                check_poll_cleanup_cases(poll_cases)
            registry_cases = [] if args.baseline else manifest["registry_deadline_cases"]
            if not args.baseline:
                check_registry_deadline_cases(registry_cases)
            missing_name_cases = [] if args.baseline else manifest["missing_name_cases"]
            if not args.baseline:
                check_missing_name_cases(missing_name_cases)
            atomic_root_cases = [] if args.baseline else manifest["atomic_root_cases"]
            if not args.baseline:
                check_atomic_root_cases(atomic_root_cases)
            connect_cases = [] if args.baseline else manifest["prepared_connect_cases"]
            if not args.baseline:
                check_prepared_connect_cases(connect_cases)
            prepared_job_cases = [] if args.baseline else manifest["prepared_job_cases"]
            if not args.baseline:
                check_prepared_job_cases(prepared_job_cases)
            fence_cases = [] if args.baseline else manifest["fence_priority_cases"]
            if not args.baseline:
                check_fence_priority_cases(fence_cases)
            environment = os.environ.copy()
            environment["CARGO_TARGET_DIR"] = str(output.parent / "windows-qualification-target")
            shared.invoke(["cargo", "test", "--locked", "-p", "diskgraph-engine", "--lib", "--no-run", "--message-format=json"],
                          source, output, "build-cleanup-fixtures", environment)
            binaries = []
            for line in (output / "build-cleanup-fixtures.stdout").read_text(encoding="utf-8").splitlines():
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
            for case in list(prerequisites) + list(cases) + list(birth_cases) + list(regression_cases) + list(probe_cases) + list(directory_cases) + list(resource_cases) + list(runner_cases) + list(poll_cases) + list(registry_cases) + list(missing_name_cases) + list(atomic_root_cases) + list(connect_cases) + list(prepared_job_cases) + list(fence_cases) + list(enumeration_cases):
                name = case.rsplit("::", 1)[-1]
                try:
                    shared.invoke([str(binary), case, "--exact", "--nocapture", "--test-threads=1"],
                                  source, output, name, environment, timeout=120)
                    passed = "test result: ok. 1 passed; 0 failed;" in (output / (name + ".stdout")).read_text(encoding="utf-8")
                except RuntimeError:
                    passed = False
                results.append({"case": case, "passed": passed, "phase": "io_prerequisite" if case in prerequisites else "birth" if case in birth_cases else "regression" if case in regression_cases else "probe" if case in probe_cases else "directory_prerequisite" if case == DIRECTORY_CASES[0] else "directory" if case in directory_cases else "resource" if case in resource_cases else "runner" if case in runner_cases else "poll_cleanup" if case in poll_cases else "registry_deadline" if case in registry_cases else "missing_name" if case in missing_name_cases else "atomic_root" if case in atomic_root_cases else "cleanup"})
                if case == DIRECTORY_CASES[0] and not passed:
                    raise RuntimeError("actual private directory prerequisite failed; directory recovery RED not qualified")
                if case == prerequisites[-1] and not all(result["passed"] for result in results):
                    raise RuntimeError("pending IO prerequisites failed; cleanup qualification not started")
            receipt["cases"] = results
            for name, expected in manifest["sources"].items():
                if shared.digest(source / name) != expected:
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
