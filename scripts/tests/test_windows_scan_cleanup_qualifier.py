"""Windows验收驱动的清单与平台边界；不替代原生子进程测试。"""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location("windows_qualifier", SCRIPTS / "qualify_windows_scan_cleanup.py")
qualifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualifier)


class WindowsCleanupQualifierTests(unittest.TestCase):
    def test_actual_last_close_delete_requires_exact_case_and_native_marker(self):
        case = qualifier.DIRECTORY_ENUMERATION_CASES[3]
        success = ("running 1 test\n" + case + "\n"
                   "DG_VERIFIED_CHILD_AND_ROOT_LAST_CLOSE_DELETE=1\n"
                   "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n")
        self.assertTrue(qualifier.native_case_passed(case, success))
        for invalid in (
            success.replace("DG_VERIFIED_CHILD_AND_ROOT_LAST_CLOSE_DELETE=1", ""),
            success.replace(case, "another::test"),
            success.replace("1 passed; 0 failed", "0 passed; 0 failed"),
            success.replace("0 ignored", "1 ignored"),
        ):
            self.assertFalse(qualifier.native_case_passed(case, invalid))

    def test_exact_native_inventory_rejects_zero_duplicate_or_extra_cases(self):
        valid = list(qualifier.REQUIRED_CASES)
        qualifier.check_cases(valid)
        for invalid in ([], valid[:2], valid + [valid[0]], [valid[0]] * 3,
                        ["native_child::windows::windows_cleanup_recovery_tests::fake"] * 3):
            with self.assertRaises(ValueError):
                qualifier.check_cases(invalid)

    def test_transfer_prerequisites_cannot_be_missing_or_replaced(self):
        valid = list(qualifier.IO_PREREQUISITE_CASES)
        qualifier.check_prerequisite_cases(valid)
        for cases in ([], valid[:-1], valid + [valid[0]], list(qualifier.REQUIRED_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_prerequisite_cases(cases)

    def test_real_frozen_windows_sources_mount_and_match(self):
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory)
            source = SCRIPTS.parent / qualifier.CANDIDATE
            destination = checkout / qualifier.CANDIDATE
            destination.mkdir(parents=True)
            for name in ("candidate.tar.gz", "manifest.json"):
                (destination / name).write_bytes((source / name).read_bytes())
            manifest = qualifier.shared.mount(checkout, qualifier.CANDIDATE)
            self.assertEqual(len(manifest["sources"]), 498)
            qualifier.check_directory_enumeration_cases(manifest["directory_enumeration_cases"])
            qualifier.check_prepared_connect_cases(manifest["prepared_connect_cases"])
            qualifier.check_prepared_job_cases(manifest["prepared_job_cases"])
            qualifier.check_cases(manifest["cleanup_cases"])
            qualifier.check_prerequisite_cases(manifest["io_prerequisite_cases"])
            qualifier.check_birth_cases(manifest["birth_cases"])
            qualifier.check_regression_cases(manifest["regression_cases"])
            qualifier.check_probe_cases(manifest["probe_cases"])
            qualifier.check_directory_cases(manifest["directory_cases"])
            qualifier.check_resource_cases(manifest["resource_cases"])
            qualifier.check_runner_cases(manifest["runner_cases"])
            qualifier.check_poll_cleanup_cases(manifest["poll_cleanup_cases"])
            qualifier.check_registry_deadline_cases(manifest["registry_deadline_cases"])
            qualifier.check_missing_name_cases(manifest["missing_name_cases"])
            qualifier.check_atomic_root_cases(manifest["atomic_root_cases"])
            self.assertTrue(manifest["production_cleanup_algorithm_modified"])
            for name, expected in manifest["sources"].items():
                self.assertEqual(qualifier.shared.digest(checkout / name), expected)

    def test_managed_probe_cases_cannot_be_omitted_or_replaced(self):
        valid = list(qualifier.PROBE_CASES)
        qualifier.check_probe_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.BIRTH_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_probe_cases(invalid)

    def test_private_directory_prerequisite_and_order_cannot_be_omitted(self):
        valid = list(qualifier.DIRECTORY_CASES)
        qualifier.check_directory_cases(valid)
        for invalid in ([], valid[1:], valid[::-1], valid + [valid[0]]):
            with self.assertRaises(ValueError):
                qualifier.check_directory_cases(invalid)

    def test_resource_slot_cases_cannot_be_omitted_or_substituted(self):
        valid = [
            "live_evidence::probe_resource_pool_tests::real_directory_delete_failure_keeps_session_slot_until_native_release",
            "live_evidence::probe_resource_pool_tests::borrowed_directory_prevents_session_reuse_after_budget_drop",
        ]
        qualifier.check_resource_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.PROBE_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_resource_cases(invalid)

    def test_runner_lifecycle_inventory_cannot_be_omitted(self):
        valid = ["runner::tests::" + name for name in (
            "stop_preserves_original_background_panic_payload",
            "managed_runner_rejects_recovery_from_another_pool_before_start",
            "busy_managed_runner_does_not_claim_or_fail_queued_job",
            "active_probe_session_does_not_turn_queued_job_into_capacity_failure",
            "recovered_runner_admission_does_not_remain_poisoned_after_panic",
        )]
        qualifier.check_runner_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.RESOURCE_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_runner_cases(invalid)

    def test_poll_cleanup_inventory_requires_all_five_real_owner_cases(self):
        valid = list(qualifier.POLL_CLEANUP_CASES)
        qualifier.check_poll_cleanup_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.RESOURCE_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_poll_cleanup_cases(invalid)

    def test_registry_deadline_inventory_requires_original_owner_and_lock_cases(self):
        valid = ["native_child::windows::windows_registry_deadline_tests::" + name for name in (
            "expired_registry_drain_retains_original_owner_and_capacity_until_actual_retry",
            "registry_deadline_wait_error_preserves_original_owner_and_capacity",
            "registry_deadline_query_error_preserves_original_owner_and_capacity",
        )] + ["scan_worker_registry::windows_registry_lock_tests::contended_registry_drain_returns_pending_without_taking_reserved_slot"]
        qualifier.check_registry_deadline_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.POLL_CLEANUP_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_registry_deadline_cases(invalid)

    def test_missing_name_cases_cannot_be_omitted_or_replaced(self):
        valid = ["live_evidence::git_private_directory_owner_tests::" + name for name in (
            "moved_original_directory_is_not_complete_when_original_path_is_missing",
            "foreign_replacement_is_retained_until_original_directory_returns",
        )]
        qualifier.check_missing_name_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.RESOURCE_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_missing_name_cases(invalid)

    def test_atomic_root_cases_require_real_git_and_junction_controls(self):
        valid = ["live_evidence::windows_git_private_root_tests::" + name for name in (
            "atomic_root_create_refuses_existing_directory_without_adopting_it",
            "moved_root_delete_reopen_targets_original_object_not_foreign_replacement",
            "original_anchor_allows_real_managed_git_cwd_and_unmodified_directory_lease",
            "moved_root_reopen_ignores_junction_replacement_and_preserves_external_target",
            "original_share_read_lease_blocks_delete_reopen_until_released",
            "held_parent_lease_blocks_move_until_creation_phase_is_released",
        )]
        qualifier.check_atomic_root_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.MISSING_NAME_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_atomic_root_cases(invalid)

    def test_birth_owner_cases_cannot_be_omitted(self):
        valid = list(qualifier.BIRTH_CASES)
        qualifier.check_birth_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.REQUIRED_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_birth_cases(invalid)

    def test_original_baseline_preserves_frozen_sources_and_case_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            checkout = Path(temporary)
            source = SCRIPTS.parent / qualifier.BASELINE_CANDIDATE
            destination = checkout / qualifier.BASELINE_CANDIDATE
            destination.mkdir(parents=True)
            for name in ("candidate.tar.gz", "manifest.json"):
                (destination / name).write_bytes((source / name).read_bytes())
            manifest = qualifier.shared.mount(checkout, qualifier.BASELINE_CANDIDATE)
            self.assertEqual(len(manifest["sources"]), 461)
            self.assertFalse(manifest["production_birth_algorithm_modified"])
            qualifier.check_cases(manifest["cleanup_cases"])
            qualifier.check_prerequisite_cases(manifest["io_prerequisite_cases"])
            qualifier.check_birth_cases(manifest["birth_cases"])
            qualifier.check_regression_cases(manifest["regression_cases"])
            for name, expected in manifest["sources"].items():
                self.assertEqual(qualifier.shared.digest(checkout / name), expected)

    def test_platform_gate_refuses_local_mac_and_non_ci_windows(self):
        for platform, ci, runner in (("Darwin", "true", "github-hosted"),
                                     ("Windows", "false", "github-hosted"),
                                     ("Windows", "true", "self-hosted")):
            with self.assertRaises(RuntimeError):
                qualifier.check_platform(platform, ci, runner)
        qualifier.check_platform("Windows", "true", "github-hosted")


if __name__ == "__main__":
    unittest.main()


class PreparedConnectInventoryTests(unittest.TestCase):
    def test_actual_connect_inventory_rejects_missing_duplicate_and_substitute(self):
        valid = list(qualifier.PREPARED_CONNECT_CASES)
        qualifier.check_prepared_connect_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], [valid[0]] * len(valid),
                        list(qualifier.IO_PREREQUISITE_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_prepared_connect_cases(invalid)


class PreparedJobInventoryTests(unittest.TestCase):
    def test_prepared_job_requires_original_io_capacity_and_actual_create_failure(self):
        valid = list(qualifier.PREPARED_JOB_CASES)
        qualifier.check_prepared_job_cases(valid)
        for invalid in ([], valid[:-1], valid + [valid[0]], list(qualifier.PREPARED_CONNECT_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_prepared_job_cases(invalid)


class PreparedBirthBaselineGuardTests(unittest.TestCase):
    def test_only_original_lost_owner_failure_is_target_red(self):
        import qualify_windows_prepared_birth_baseline as baseline
        stdout = "DG_ACTUAL_CREATE_PROCESS_FAILURE=267\ntest " + baseline.CASE + " ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;"
        stderr = "actual failed CreateProcess must retain original Job"
        baseline.check_target_red(stdout, stderr)
        for out, err in (("", stderr), (stdout, "unrelated failure"),
                         (stdout.replace("1 failed", "0 failed"), stderr),
                         (stdout.replace("0 ignored", "1 ignored"), stderr)):
            with self.assertRaises(RuntimeError):
                baseline.check_target_red(out, err)


class PreparedBirthBaselineArchiveTests(unittest.TestCase):
    def test_baseline_build_uses_complete_historical_workspace_without_mutating_product(self):
        import hashlib
        import os
        import subprocess
        from unittest.mock import patch
        import qualify_windows_prepared_birth_baseline as baseline
        checkout = SCRIPTS.parent
        product_manifest = checkout / "crates/diskgraph-cli/Cargo.toml"
        before = product_manifest.read_bytes()
        for candidate in (baseline.CANDIDATE, qualifier.CANDIDATE, qualifier.BASELINE_CANDIDATE):
            git_environment = {
                "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.autocrlf",
                "GIT_CONFIG_VALUE_0": "true",
            }
            with self.subTest(candidate=candidate), patch.dict(os.environ, git_environment), baseline.isolated_source(checkout, candidate) as (source, manifest):
                self.assertNotEqual(source, checkout)
                original = subprocess.check_output([
                    "git", "show", manifest["base_ref"] + ":crates/diskgraph-cli/Cargo.toml"
                ], cwd=checkout)
                self.assertEqual((source / "crates/diskgraph-cli/Cargo.toml").read_bytes(), original)
                self.assertNotEqual(original, before)
                for name, expected in manifest["sources"].items():
                    self.assertEqual(hashlib.sha256((source / name).read_bytes()).hexdigest(), expected)
                subprocess.run(["cargo", "metadata", "--locked", "--format-version=1"],
                               cwd=source, check=True, stdout=subprocess.DEVNULL)
                # 真实反例重建 CI 的混合 workspace；必须拒绝锁文件不一致。
                (source / "crates/diskgraph-cli/Cargo.toml").write_bytes(before)
                mixed = subprocess.run(["cargo", "metadata", "--locked", "--format-version=1"],
                                       cwd=source, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.PIPE, text=True)
                self.assertNotEqual(mixed.returncode, 0)
                self.assertIn("cannot update the lock file", mixed.stderr)
            self.assertFalse(source.exists())
        self.assertEqual(product_manifest.read_bytes(), before)

    def test_baseline_mounts_exact_original_sources_with_native_regression(self):
        import qualify_windows_prepared_birth_baseline as baseline
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory)
            source = SCRIPTS.parent / baseline.CANDIDATE
            destination = checkout / baseline.CANDIDATE
            destination.mkdir(parents=True)
            for name in ("candidate.tar.gz", "manifest.json"):
                (destination / name).write_bytes((source / name).read_bytes())
            manifest = qualifier.shared.mount(checkout, baseline.CANDIDATE)
            self.assertEqual(len(manifest["sources"]), 491)
            self.assertEqual(manifest["baseline_expected_failure_case"], baseline.CASE)


class PreparedBirthInterleavedOutputTests(unittest.TestCase):
    def test_native_marker_may_interleave_with_named_test_line(self):
        import qualify_windows_prepared_birth_baseline as baseline
        stderr = "actual failed CreateProcess must retain original Job"
        stdout = "test " + baseline.CASE + " ... DG_ACTUAL_CREATE_PROCESS_FAILURE=267\nFAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;"
        baseline.check_target_red(stdout, stderr)
        for invalid in (stdout.replace("=267", "=unknown"), stdout.replace(baseline.CASE, "unrelated_case")):
            with self.assertRaises(RuntimeError):
                baseline.check_target_red(invalid, stderr)


class ExpiredProbeBaselineGuardTests(unittest.TestCase):
    def test_only_actual_expired_cleanup_boundary_is_red(self):
        import qualify_windows_prepared_birth_baseline as baseline
        stdout = "test " + baseline.DEADLINE_CASE + " ... DG_EXPIRED_PROBE_BOUNDARY=1\nFAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;"
        stderr = "expired product probe must not enter legacy wait"
        baseline.check_deadline_red(stdout, stderr)
        for out, err in ((stdout.replace("=1", "=0"), stderr),
                         (stdout.replace(baseline.DEADLINE_CASE, "other"), stderr),
                         (stdout.replace("1 failed", "0 failed"), stderr),
                         (stdout, "birth missing")):
            with self.assertRaises(RuntimeError):
                baseline.check_deadline_red(out, err)


class FencePriorityInventoryTests(unittest.TestCase):
    def test_requires_all_original_priority_and_independent_request_cases(self):
        cases = ["process_fence_priority_tests::" + name for name in (
            "live_public_claim_and_original_authority_reach_fence_work",
            "original_expiry_remains_authority_denied_when_execution_deadline_already_elapsed",
            "current_owner_and_live_metadata_grant_still_gate_unexpired_work",
            "wrong_owner_is_rejected_before_unexpired_fence_work",
        )]
        qualifier.check_fence_priority_cases(cases)
        for invalid in ([], cases[:-1], cases + [cases[0]], list(qualifier.PROBE_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_fence_priority_cases(invalid)


class DirectoryEnumerationInventoryTests(unittest.TestCase):
    def test_original_enumeration_and_decoder_cases_cannot_be_missing_or_duplicated(self):
        cases = list(qualifier.DIRECTORY_ENUMERATION_CASES)
        qualifier.check_directory_enumeration_cases(cases)
        for invalid in ([], cases[:-1], cases + [cases[0]], list(qualifier.RESOURCE_CASES)):
            with self.assertRaises(ValueError):
                qualifier.check_directory_enumeration_cases(invalid)

    def test_directory_baseline_requires_exact_runtime_assertion_not_build_or_zero_tests(self):
        import qualify_windows_directory_enumeration as baseline
        stdout = ("test " + baseline.CASE + " ... DG_HELD_DIRECTORY_ARGUMENT_CONTROL=1\n"
                  "test result: FAILED. 0 passed; 1 failed; 0 ignored;\n")
        stderr = "held directory must never enumerate the foreign argument path"
        baseline.check_target_red(stdout, stderr)
        for out, err in (("", "compile error"), (stdout.replace(baseline.CASE, "wrong_case"), stderr),
                         (stdout.replace("DG_HELD_DIRECTORY_ARGUMENT_CONTROL=1", ""), stderr),
                         (stdout.replace("1 failed", "0 failed"), stderr), (stdout, "unrelated failure")):
            with self.assertRaises(RuntimeError):
                baseline.check_target_red(out, err)
