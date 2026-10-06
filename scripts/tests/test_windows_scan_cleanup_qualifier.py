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
            self.assertEqual(len(manifest["sources"]), 491)
            qualifier.check_prepared_connect_cases(manifest["prepared_connect_cases"])
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
