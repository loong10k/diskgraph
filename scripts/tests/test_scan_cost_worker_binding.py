"""真实文件验证每侧release helper绑定；不代替Linux扫描性能验收。"""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "accept-linux-scan-namespace-cost.py"
spec = importlib.util.spec_from_file_location("cost", SCRIPT)
cost = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cost)


class WorkerBindingTests(unittest.TestCase):
    def test_worker_binding_rejects_replaced_bytes_and_wrong_length(self):
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "worker"
            image.write_bytes(b"built release helper")
            source = {"worker": {"binary": str(image), "binary_sha256": hashlib.sha256(image.read_bytes()).hexdigest(), "bytes": image.stat().st_size}}
            values = cost.worker_environment(source)
            self.assertEqual(values["DISKGRAPH_SCAN_WORKER_PATH"], str(image))
            self.assertEqual(values["DISKGRAPH_SCAN_WORKER_SHA256"], source["worker"]["binary_sha256"])
            source["worker"]["bytes"] += 1
            with self.assertRaises(RuntimeError):
                cost.worker_environment(source)
            source["worker"]["bytes"] -= 1
            image.write_bytes(b"wrong release helper")
            with self.assertRaises(RuntimeError):
                cost.worker_environment(source)

    def test_historical_side_has_no_worker_environment(self):
        self.assertEqual(cost.worker_environment({"worker": None}), {})


class MemoryReportTests(unittest.TestCase):
    def test_missing_observation_never_becomes_zero_difference(self):
        for candidate, baseline in ((None, 10), (10, None), (None, None)):
            with self.subTest(candidate=candidate, baseline=baseline):
                self.assertIsNone(cost.memory_difference(candidate, baseline))

    def test_valid_high_water_difference_can_be_negative(self):
        self.assertEqual(cost.memory_difference(10, 20), -10)
        self.assertEqual(cost.memory_difference(20, 10), 10)

    def test_invalid_observation_is_unknown(self):
        for candidate in (-1, True, "10", 1.5):
            with self.subTest(candidate=candidate):
                self.assertIsNone(cost.memory_difference(candidate, 10))

    def test_archived_harness_includes_memory_module(self):
        self.assertIn("crates/diskgraph-engine/tests/benchmark_support/peak_memory.rs", cost.HARNESS)
