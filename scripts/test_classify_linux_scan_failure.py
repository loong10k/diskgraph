"""原配对失败回执与保守分类边界；未知不能误报旧基线失败。"""
import gzip
import json
from pathlib import Path
import unittest
from classify_linux_scan_failure import failure_side


class FailureSideTests(unittest.TestCase):
    def test_original_candidate_conflict_does_not_request_baseline_diagnostic(self):
        receipt = Path(__file__).resolve().parents[1] / "docs/benchmarks/linux_paired_candidate_conflict_6c/receipt.json.gz"
        with gzip.open(receipt, "rt") as source:
            original = json.load(source)
        self.assertEqual(original["status"], "failed")
        self.assertEqual(original["commands"][-1]["label"], "200000-wide-round2-candidate")
        self.assertEqual(failure_side(original), "candidate")

    def test_expected_object_fetch_failure_does_not_override_final_candidate(self):
        receipt = {"status": "failed", "commands": [{"label": "baseline-object", "exit_code": 128}, {"label": "200000-wide-round2-candidate", "exit_code": 101}]}
        self.assertEqual(failure_side(receipt), "candidate")

    def test_actual_baseline_run_and_metric_failure_are_classified(self):
        for code in [0, 101, None]:
            self.assertEqual(failure_side({"status": "failed", "commands": [{"label": "200000-wide-round1-baseline", "exit_code": code}]}), "baseline")

    def test_baseline_build_failure_is_separate_from_expected_fetch(self):
        self.assertEqual(failure_side({"status": "failed", "commands": [{"label": "baseline-build", "exit_code": 101}]}), "baseline")
        self.assertEqual(failure_side({"status": "failed", "commands": [{"label": "baseline-object", "exit_code": 128}]}), "unknown")

    def test_missing_or_unrecognized_phase_is_unknown(self):
        for commands in [[], [{"label": "filesystem"}], [{"label": "candidate-not-a-real-phase"}], [None]]:
            self.assertEqual(failure_side({"status": "failed", "commands": commands}), "unknown")

    def test_passed_and_aborted_runs_do_not_diagnose_failure(self):
        for status in ["passed", "running", "aborted"]:
            self.assertEqual(failure_side({"status": status}), "none")


if __name__ == "__main__":
    unittest.main()
