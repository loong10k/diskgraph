"""Windows准备诊断只记录真实成本，不改变正式负载门禁。"""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("fixture_diagnostic", ROOT / "scripts/diagnose_windows_fixture_workers.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FixtureWorkerDiagnosticTests(unittest.TestCase):
    def test_real_small_cases_preserve_order_coverage_and_source_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "receipt.json"
            result = MODULE.run(output, files=103)
            self.assertEqual(result, json.loads(output.read_text()))
            self.assertEqual(result["status"], "complete")
            self.assertEqual([case["workers"] for case in result["cases"]], [1, 2, 4, 4, 2, 1])
            self.assertTrue(all(case["files"] == 103 for case in result["cases"]))
            self.assertTrue(all(case["create_seconds"] >= 0 and case["cleanup_seconds"] >= 0
                                for case in result["cases"]))
            self.assertEqual(len(result["source_sha256"]), 2)

    def test_actual_write_failure_is_preserved_and_cannot_report_complete(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "receipt.json"
            failure = OSError("fixture write failed")
            with patch.object(MODULE.LOAD, "create_fixture", side_effect=failure):
                with self.assertRaises(OSError) as caught:
                    MODULE.run(output, files=103)
            self.assertIs(caught.exception, failure)
            result = json.loads(output.read_text())
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["cases"], [])

    def test_receipt_write_error_does_not_mask_original_diagnostic_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            failure = OSError("original fixture failure")
            with patch.object(MODULE, "measure_case", side_effect=failure), \
                    patch.object(Path, "write_text", side_effect=[1, OSError("receipt disk full")]):
                with self.assertRaises(OSError) as caught:
                    MODULE.run(Path(temporary) / "receipt.json", files=103)
            self.assertIs(caught.exception, failure)

    def test_coverage_refuses_incomplete_fixture_even_when_writer_returns(self):
        with patch.object(MODULE.LOAD, "create_fixture", return_value=None):
            with self.assertRaisesRegex(RuntimeError, "count mismatch"):
                MODULE.measure_case(1, 103)


if __name__ == "__main__":
    unittest.main()
