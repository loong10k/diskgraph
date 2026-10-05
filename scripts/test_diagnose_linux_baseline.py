#!/usr/bin/env python3
"""诊断工具合同测试：真实旧源可逆覆盖、真实子进程有界捕获；不编译或测量 Rust。"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("baseline_diagnostic", REPO / "scripts/diagnose-linux-baseline.py")
DIAG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DIAG)
HARNESS = DIAG.load_harness(REPO)


class DiagnosticContractTests(unittest.TestCase):
    """只读真实固定提交与本测试自建子进程，保留失败和完整输出前缀。"""

    def test_exact_old_sources_have_only_reversible_four_site_patch(self):
        phases = []
        for name in DIAG.SOURCE_HASHES:
            old = subprocess.check_output(["git", "show", f"{DIAG.BASELINE}:{name}"], cwd=REPO)
            changed, difference = DIAG.patch_source(name, old)
            self.assertTrue(difference)
            restored = changed.decode()
            for before, after in reversed(DIAG.replacements(name)):
                self.assertEqual(restored.count(after), 1)
                restored = restored.replace(after, before, 1)
            self.assertEqual(restored.encode(), old)
            phases.extend(line for line in changed.decode().splitlines() if DIAG.PREFIX in line)
            self.assertIn("scan_started.elapsed().as_micros()", changed.decode())
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                DIAG.patch_source(name, old + b"\n")
        self.assertEqual(len(phases), 4)
        for phase in ("keeper_checked", "keeper_join_panic", "walk_heartbeat", "walk_observation_guard"):
            self.assertEqual(sum(f"phase={phase} " in line for line in phases), 1)

    def command(self, output, code, cap=65536, timeout=5):
        report = {"commands": [], "kind": "diagnostic-only"}
        result = DIAG.run_command([sys.executable, "-c", code], output, output, "fixture",
                                  report, HARNESS, cap=cap, timeout=timeout)
        return result, report

    def test_success_keeps_exact_two_streams_and_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            result, report = self.command(output, "import os;os.write(1,b'out\\x00');os.write(2,b'err\\xff')")
            self.assertEqual(result["exit_code"], 0)
            for name, expected in (("stdout", b"out\0"), ("stderr", b"err\xff")):
                self.assertEqual((output / f"fixture.{name}").read_bytes(), expected)
                self.assertEqual(result[name]["sha256"], DIAG.digest(expected))
            self.assertEqual(json.loads((output / "receipt.json").read_text()), report)
            self.assertNotIn("measurements", report)
            self.assertNotIn("pairs", report)

    def test_nonzero_exit_stays_failed_without_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            result, report = self.command(Path(directory), "import sys;print('original failure',file=sys.stderr);sys.exit(101)")
            self.assertEqual(result["exit_code"], 101)
            self.assertEqual(len(report["commands"]), 1)
            with self.assertRaisesRegex(RuntimeError, "exited 101; no retry"):
                DIAG.require_success(result)

    def test_capture_overflow_preserves_bounded_raw_prefix_and_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            with self.assertRaises(OverflowError):
                self.command(output, "import os,time;os.write(2,b'x'*65536);time.sleep(30)", cap=1024)
            record = json.loads((output / "receipt.json").read_text())["commands"][0]
            self.assertEqual((output / "fixture.stderr").read_bytes(), b"x" * 1024)
            self.assertEqual(record["truncated_stream"], "stderr")
            self.assertTrue(record["terminated_process_group"])
            self.assertNotEqual(record["exit_code"], 0)

    def test_timeout_does_not_turn_into_success_and_reaps_direct_child(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            with self.assertRaises(TimeoutError):
                self.command(output, "import os,time;os.write(1,b'ready');time.sleep(30)", timeout=0.5)
            record = json.loads((output / "receipt.json").read_text())["commands"][0]
            self.assertEqual((output / "fixture.stdout").read_bytes(), b"ready")
            self.assertTrue(record["terminated_process_group"])
            self.assertNotEqual(record["exit_code"], 0)
            with self.assertRaises(ChildProcessError):
                os.waitpid(record["process_group_id"], os.WNOHANG)

    def test_real_timeout_survives_receipt_failure_and_retains_reaped_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            report = {"commands": []}
            secondary = OSError("receipt disk failure")
            with mock.patch.object(HARNESS, "save", side_effect=secondary):
                with self.assertRaisesRegex(TimeoutError, "original command deadline expired"):
                    DIAG.run_command([sys.executable, "-c", "import time;time.sleep(30)"],
                                     output, output, "timeout", report, HARNESS, cap=1024, timeout=0.1)
            record = report["commands"][0]
            self.assertTrue(record["terminated_process_group"])
            self.assertNotEqual(record["exit_code"], 0)
            self.assertEqual(record["secondary_errors"][0]["phase"], "receipt_save")
            with self.assertRaises(ChildProcessError):
                os.waitpid(record["process_group_id"], os.WNOHANG)

    def test_primary_object_survives_raw_close_hash_and_receipt_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            report = {"commands": []}
            primary = KeyboardInterrupt("original interrupt sentinel")
            original_open = Path.open
            real_files = []

            def opening(path, *args, **kwargs):
                handle = original_open(path, *args, **kwargs)
                if args and args[0] == "wb":
                    real_files.append(handle)
                    wrapper = mock.Mock(wraps=handle)
                    def close():
                        handle.close()
                        raise OSError("raw close sentinel")
                    wrapper.close.side_effect = close
                    return wrapper
                return handle

            with mock.patch.object(Path, "open", opening), \
                    mock.patch.object(DIAG.selectors.DefaultSelector, "select", side_effect=primary), \
                    mock.patch.object(HARNESS, "sha", side_effect=OSError("hash sentinel")), \
                    mock.patch.object(HARNESS, "save", side_effect=OSError("save sentinel")):
                try:
                    DIAG.run_command([sys.executable, "-c", "import time;time.sleep(30)"],
                                     output, output, "interrupt", report, HARNESS, cap=1024, timeout=5)
                except BaseException as actual:
                    self.assertIs(actual, primary)
                else:
                    self.fail("original interrupt was lost")
            self.assertTrue(all(handle.closed for handle in real_files))
            record = report["commands"][0]
            self.assertTrue(record["terminated_process_group"])
            self.assertEqual([entry["phase"] for entry in record["secondary_errors"]],
                             ["stdout_close", "stderr_close", "stdout_hash", "stderr_hash", "receipt_save"])

    def test_successful_command_reports_finalize_error_as_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            primary = OSError("only receipt failure")
            with mock.patch.object(HARNESS, "save", side_effect=primary):
                try:
                    self.command(Path(directory), "pass")
                except OSError as actual:
                    self.assertIs(actual, primary)
                else:
                    self.fail("finalization failure was accepted")

    def test_real_failed_exit_stays_primary_when_receipt_write_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.object(HARNESS, "save", side_effect=OSError("receipt failure")):
                result, _ = self.command(Path(directory), "import sys;sys.exit(101)")
            self.assertEqual(result["exit_code"], 101)
            self.assertEqual(result["secondary_errors"][0]["phase"], "receipt_save")
            with self.assertRaisesRegex(RuntimeError, "exited 101"):
                DIAG.require_success(result)

    def test_main_preserves_actual_prepare_io_error_when_archiving_and_saving_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            actual_errors, reports = [], []
            original_sha = HARNESS.sha

            def prepare_failure(repo, work, destination, report, harness):
                reports.append(report)
                (destination / "raw.txt").write_bytes(b"existing evidence")
                try:
                    (work / "real-missing-source").read_bytes()
                except FileNotFoundError as error:
                    actual_errors.append(error)
                    raise

            def failing_artifact_hash(path):
                if path.name == "raw.txt":
                    raise OSError("archive hash failure")
                return original_sha(path)

            with mock.patch.object(sys, "argv", ["diagnose", "--prepare-only", "--output-dir", str(output)]), \
                    mock.patch.object(DIAG, "load_harness", return_value=HARNESS), \
                    mock.patch.object(DIAG, "prepare", side_effect=prepare_failure), \
                    mock.patch.object(HARNESS, "sha", side_effect=failing_artifact_hash), \
                    mock.patch.object(HARNESS, "save", side_effect=OSError("archive receipt failure")):
                try:
                    DIAG.main()
                except FileNotFoundError as actual:
                    self.assertIs(actual, actual_errors[0])
                else:
                    self.fail("actual preparation error was lost")
            self.assertEqual(reports[0]["status"], "failed")
            self.assertEqual([item["phase"] for item in reports[0]["secondary_errors"]],
                             ["raw.txt_hash", "receipt_save"])


if __name__ == "__main__":
    unittest.main()
