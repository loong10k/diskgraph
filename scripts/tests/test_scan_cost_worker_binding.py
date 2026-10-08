"""真实文件验证每侧release helper绑定；不代替Linux扫描性能验收。"""
import hashlib
import os
import sys
import time
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

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


@unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
class FailedCommandCleanupTests(unittest.TestCase):
    def test_nonzero_command_stops_descendant_before_late_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / "late-write"
            ready = root / "ready"
            child = ("from pathlib import Path; import time; "
                     f"Path({str(ready)!r}).touch(); time.sleep(0.5); "
                     f"Path({str(marker)!r}).touch()")
            parent = ("import subprocess, sys, time; from pathlib import Path; "
                      f"subprocess.Popen([sys.executable, '-c', {child!r}]); "
                      f"ready=Path({str(ready)!r}); "
                      "exec('while not ready.exists(): time.sleep(0.005)'); sys.exit(7)")
            report = {"commands": []}
            with self.assertRaises(RuntimeError):
                cost.command([sys.executable, "-c", parent], root, root,
                             "failed", report, timeout=5)
            self.assertTrue(ready.exists())
            time.sleep(0.7)
            self.assertFalse(marker.exists(), "failed measurement left a running descendant")
            self.assertEqual(report["commands"][0]["exit_code"], 7)

    def test_group_signal_precedes_leader_reap(self):
        process = mock.Mock(pid=123, returncode=None)
        events = []
        process.wait.side_effect = lambda: events.append("reap")
        with mock.patch.object(cost.os, "killpg", side_effect=lambda *args: events.append("signal")):
            cost.terminate_group(process)
        self.assertEqual(events, ["signal", "reap"])

    def test_reaped_leader_never_receives_group_signal(self):
        process = mock.Mock(pid=123, returncode=7)
        with mock.patch.object(cost.os, "killpg") as signal_group:
            with self.assertRaises(RuntimeError):
                cost.terminate_group(process)
            signal_group.assert_not_called()

    def test_exit_observation_preserves_waitable_leader(self):
        import subprocess
        process = subprocess.Popen([sys.executable, "-c", "raise SystemExit(7)"],
                                   start_new_session=True)
        try:
            self.assertEqual(cost.observe_exit(process, 5), 7)
            self.assertIsNone(process.returncode)
            observed = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT)
            self.assertEqual(observed.si_status, 7)
        finally:
            self.assertEqual(process.wait(), 7)
