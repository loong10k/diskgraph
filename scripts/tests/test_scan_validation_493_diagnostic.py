#!/usr/bin/env python3
"""固定 493 首错诊断组成/编排合同；不将 Python 合同视为 Linux kernel 或 Rust Drop 证明。"""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts/diagnose-linux-scan-validation-493.py"
BASELINE = "493257fda48242b2fb326be948e20c3636d228f2"
PREFIX = "DG_SCAN_VALIDATION_493_FIRST_FAILURE "
TEMPLATE = "crates/diskgraph-engine/integration_candidates/scan_validation_493_diagnostic.rs"
TARGET = "crates/diskgraph-engine/src/native_process/scan_validation_493_diagnostic.rs"
EXPECTED = {
    "crates/diskgraph-engine/src/scan_execution.rs": "92ef03e07dc19c8920258e6e9cd599ae007c112df02cc44c999ab559775ce2b2",
    "crates/diskgraph-engine/src/native_process/mod.rs": "92b2bded52bd9e54998e3808122daf7883cdc619e3404656c07a29f11576b289",
    "crates/diskgraph-engine/src/native_process/linux_scan_root.rs": "a0424783cb863ee8097dc4074516c7004da6170e6fc87be3fce8b9ca6bcdfee9",
    "crates/diskgraph-engine/src/native_process/linux_scan_namespace.rs": "86105befe2f907279f87b4f617a977fb1e70e47cdb03cc7739828e84d8f3b6ab",
    "crates/diskgraph-engine/src/native_process/linux_directory_identity.rs": "ebdcd42f2e9b6d90ffa3edb8c6306a931bd7e96460d752cdeb0f9923294f7c2a",
    "crates/diskgraph-engine/src/native_process/linux_open.rs": "b569eda8885214eb66f9fc026daa9751e504f88ac6cb7f347a5f2e0441d635d8",
}
HARNESS = (
    "crates/diskgraph-engine/tests/hardening_benchmark.rs",
    "crates/diskgraph-engine/tests/benchmark_support/mod.rs",
    "crates/diskgraph-engine/tests/benchmark_support/peak_memory.rs",
    "crates/diskgraph-engine/tests/benchmark_support/benchmark_engine.rs",
    "crates/diskgraph-engine/tests/benchmark_support/namespace_cost.rs",
    "crates/diskgraph-engine/tests/benchmark_support/scan_failure_diagnostic.rs",
)


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ScanValidation493DiagnosticTests(unittest.TestCase):
    """读取真实固定提交，核对精确覆盖和闭合诊断；源不存在只记接口缺失。"""

    def implementation(self):
        self.assertTrue(SCRIPT.is_file(), "diagnostic script absent: interface absence, not native behavior RED")
        return load(SCRIPT, "validation_diagnostic_under_test")

    def record(self, **changes):
        value = {
            "phase": "namespace_validate", "operation": "openat2",
            "native_failure": "unsupported", "errno": 11, "component": 2,
            "elapsed_us": 17000000, "validation": 3,
            "owner_matches": True, "principal_matches": True,
            "dev_equal": None, "ino_equal": None, "mount_equal": None,
            "returned_category": "business_conflict",
        }
        value.update(changes)
        return value

    def line(self, value):
        return (PREFIX + json.dumps(value, separators=(",", ":")) + "\n").encode()

    def test_new_script_exists_without_claiming_native_red(self):
        self.implementation()

    def test_fixed_revision_and_original_transport_are_not_replaced(self):
        diag = self.implementation()
        self.assertEqual(diag.BASELINE, BASELINE)
        self.assertEqual(diag.SOURCE_HASHES, EXPECTED)
        self.assertEqual(tuple(diag.HARNESS_FILES), HARNESS)
        self.assertEqual(diag.PREFIX, PREFIX)
        self.assertEqual(diag.TEMPLATE, TEMPLATE)
        self.assertEqual(diag.TARGET, TARGET)
        # 复用已审错误收尾实现，不另造一份近似的 timeout/finally 语义。
        for name in ("run_command", "finalize_report", "finalize_step", "require_success"):
            actual = Path(getattr(diag, name).__code__.co_filename).resolve()
            self.assertEqual(actual, (REPO / "scripts/diagnose-linux-baseline.py").resolve())

    def test_real_six_source_patch_is_exact_reversible_and_drift_rejected(self):
        diag = self.implementation()
        for name, expected in EXPECTED.items():
            with self.subTest(source=name):
                # 此命令由根执行测试时只读固定 Git 对象；作者不运行 Git/Cargo。
                original = subprocess.check_output(["git", "show", f"{BASELINE}:{name}"], cwd=REPO)
                self.assertEqual(hashlib.sha256(original).hexdigest(), expected)
                changed, difference = diag.patch_source(name, original)
                self.assertNotEqual(changed, original)
                self.assertTrue(difference)
                restored = changed.decode()
                for before, after in reversed(diag.replacements(name)):
                    self.assertEqual(restored.count(after), 1)
                    restored = restored.replace(after, before, 1)
                self.assertEqual(restored.encode(), original)
                with self.assertRaises(ValueError):
                    diag.patch_source(name, original + b"\n")
        with self.assertRaises(ValueError):
            diag.patch_source("crates/diskgraph-engine/src/job_execution.rs", b"unknown")

    def test_observation_patch_keeps_original_checks_flags_and_mapper_arms(self):
        diag = self.implementation()
        # 这里只核实际固定源码的机械保真；Rust 回调执行/Drop 时序须由 Linux private tests 验证。
        for name in EXPECTED:
            original = subprocess.check_output(["git", "show", f"{BASELINE}:{name}"], cwd=REPO)
            changed, _ = diag.patch_source(name, original)
            old, new = original.decode(), changed.decode()
            self.assertEqual(old.count("check()?;"), new.count("check()?;"), name)
            self.assertEqual(old.count("check()"), new.count("check()"), name)
            if name.endswith("/linux_scan_root.rs"):
                for arm in ("Err(Failure::BudgetExceeded) => Err(BusinessError::BudgetExceeded.into()),",
                            "Err(_) => Err(BusinessError::Conflict.into()),"):
                    self.assertEqual(old.count(arm), new.count(arm))
                self.assertEqual(new.count("self.namespace.verify(native_check)"), 1)
            if name.endswith("/linux_scan_namespace.rs"):
                for constraint in ("libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC",
                                   "0x08 | 0x04 | 0x02 | 0x20", "0x04 | 0x02 | 0x20"):
                    self.assertEqual(old.count(constraint), new.count(constraint))
            if name.endswith("/linux_open.rs"):
                # 原 errno mapper 的每条分类语句须保留；不能为观察 EAGAIN 添加 retry/降 flags。
                mapping = old[old.index("        Some(libc::EACCES | libc::EPERM)"):
                              old.index("        _ => Failure::Unavailable,") + len("        _ => Failure::Unavailable,")]
                self.assertIn(mapping, new)
                self.assertEqual(old.count("libc::SYS_openat2"), new.count("libc::SYS_openat2"))
                self.assertEqual(old.count("libc::statx("), new.count("libc::statx("))

    def test_cause_preserves_actual_scalars_without_reclassifying_failure(self):
        diag = self.implementation()
        for value in (
            self.record(),
            self.record(operation="statx", errno=13, native_failure="permission_denied"),
            self.record(operation="fstat", errno=9, native_failure="unavailable"),
            self.record(operation="identity", errno=None, native_failure="conflict",
                        dev_equal=True, ino_equal=False, mount_equal=True),
            self.record(operation="unknown", errno=None, component=None,
                        native_failure="unsupported"),
        ):
            with self.subTest(operation=value["operation"]):
                self.assertEqual(diag.collect_causes(self.line(value)), [value])

    def test_posterior_and_quota_words_are_not_first_native_cause(self):
        diag = self.implementation()
        raw = b'DG_SCAN_FAILURE_POSTERIOR_NON_ATOMIC {"errno":11}\nBusiness(Conflict) budget\n'
        self.assertEqual(diag.collect_causes(raw), [])
        self.assertEqual(diag.collect_causes(raw + self.line(self.record())), [self.record()])

    def test_unknown_fields_variants_and_non_scalar_payload_fail_closed(self):
        diag = self.implementation()
        invalid = [
            self.record(path="not-allowed"), self.record(inode=12),
            self.record(job_id="not-allowed"), self.record(message="not-allowed"),
            self.record(phase="other"), self.record(operation="guessed_eagain"),
            self.record(native_failure="invented"), self.record(returned_category="raw path"),
            self.record(returned_category=1), self.record(errno=True),
            self.record(errno="11"), self.record(component=-1),
            self.record(validation=0), self.record(elapsed_us=-1),
            self.record(owner_matches=1), self.record(ino_equal="false"),
        ]
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(ValueError):
                diag.collect_causes(self.line(value))

    def test_ambiguous_duplicate_or_oversized_cause_is_not_silently_truncated(self):
        diag = self.implementation()
        raw = self.line(self.record())
        for invalid in (raw + raw, PREFIX.encode() + b"not-json\n",
                        PREFIX.encode() + b"x" * 2049 + b"\n"):
            with self.subTest(length=len(invalid)), self.assertRaises(ValueError):
                diag.collect_causes(invalid)

    def test_prepare_only_is_real_archive_and_only_approved_overlay(self):
        diag = self.implementation()
        harness = diag.load_harness(REPO)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            work, output = base / "work", base / "output"
            work.mkdir()
            output.mkdir()
            report = {"commands": [], "kind": "diagnostic-only"}
            source = diag.prepare(REPO, work, output, report, harness)
            original = json.loads((output / "original-source.json").read_text())
            measured = json.loads((output / "diagnostic-source.json").read_text())
            changed = {name for name in original.keys() | measured.keys()
                       if original.get(name) != measured.get(name)}
            self.assertTrue(changed <= set(EXPECTED) | set(HARNESS) | {TARGET})
            self.assertTrue(set(EXPECTED) | {TARGET} <= changed)
            self.assertEqual((source / TARGET).read_bytes(), (REPO / TEMPLATE).read_bytes())
            self.assertEqual(report["source"]["commit"], BASELINE)
            self.assertEqual(report["source"]["diagnostic_before"], EXPECTED)
            self.assertEqual(report["source"]["lock_sha256"], harness.sha(source / "Cargo.lock"))
            self.assertEqual(report["source"]["archive_sha256"],
                             harness.sha(output / "baseline-archive.stdout"))
            self.assertNotIn("pairs", report)
            self.assertNotIn("measurements", report)
            self.assertEqual(len(report["commands"]), 1)
            self.assertEqual(report["commands"][0]["argv"],
                             ["git", "archive", "--format=tar", BASELINE])

    def test_run_nonzero_preserves_original_failure_and_has_one_attempt(self):
        diag = self.implementation()
        harness = diag.load_harness(REPO)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            report = {"commands": [], "kind": "diagnostic-only"}
            result = diag.run_command(
                [sys.executable, "-c", "import sys;print('original Conflict',file=sys.stderr);sys.exit(101)"],
                output, output, "case", report, harness, cap=4096, timeout=5)
            self.assertEqual(result["exit_code"], 101)
            self.assertEqual(len(report["commands"]), 1)
            with self.assertRaisesRegex(RuntimeError, "exited 101; no retry"):
                diag.require_success(result)
            self.assertEqual(diag.collect_causes((output / "case.stderr").read_bytes()), [])

    def test_execute_schedules_one_fixed_200k_attempt_and_keeps_failure(self):
        diag = self.implementation()
        harness = diag.load_harness(REPO)
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source, work, output = (base / part for part in ("source", "work", "output"))
            for path in (source, work, output):
                path.mkdir()
            report = {"commands": [], "kind": "diagnostic-only"}
            calls, fixture_calls = [], []

            def arranged_command(argv, cwd, destination, label, receipt, shared, *, cap, timeout, env=None):
                # 只验证编排 argv；不执行假 ELF，不把这些合成返回视为 native 资格。
                calls.append((list(argv), cap, timeout, dict(env or {})))
                code = 0
                if argv[0] == "cargo":
                    binary = Path(env["CARGO_TARGET_DIR"]) / "release/deps/hardening_benchmark"
                    binary.parent.mkdir(parents=True)
                    binary.write_bytes(b"orchestration-only-nonexecutable")
                    message = {"reason": "compiler-artifact", "target": {"name": "hardening_benchmark"},
                               "executable": str(binary)}
                    (destination / f"{label}.stdout").write_text(json.dumps(message) + "\n")
                elif "measure_isolated_release_fixtures" in argv:
                    code = 101
                    (destination / f"{label}.stderr").write_bytes(self.line(self.record()))
                result = {"label": label, "exit_code": code}
                receipt["commands"].append(result)
                return result

            def arranged_fixture(root, count, shape):
                fixture_calls.append((root, count, shape))
                root.mkdir()
                return {"orchestration_only": True}

            with mock.patch.object(diag, "run_command", side_effect=arranged_command), \
                    mock.patch.object(harness, "fixture", side_effect=arranged_fixture), \
                    mock.patch.object(harness, "verify_fixture"):
                with self.assertRaisesRegex(RuntimeError, "exited 101; no retry"):
                    diag.execute(source, work, output, report, harness)
            self.assertEqual([(count, shape) for _, count, shape in fixture_calls], [(200000, "wide")])
            runs = [call for call in calls if "measure_isolated_release_fixtures" in call[0]]
            self.assertEqual(len(runs), 1)
            argv, cap, timeout, environment = runs[0]
            self.assertEqual(argv[1:], ["--exact", "measure_isolated_release_fixtures", "--ignored",
                                        "--nocapture", "--test-threads=1"])
            self.assertEqual((cap, timeout), (4 * 1024 * 1024, 1200))
            self.assertEqual(environment["DG_MEASURE_CASE"], "200000")
            self.assertEqual(environment["DG_MEASURE_SHAPE"], "wide")
            self.assertFalse(Path(environment["DG_MEASURE_DATA"]).exists())
            self.assertEqual(report["measurement_exit_code"], 101)
            self.assertEqual(report["causes"], [self.record()])
            self.assertTrue(report["cause_observed"])
            self.assertNotIn("pairs", report)
            self.assertNotIn("measurements", report)

    def test_main_prepare_primary_survives_artifact_and_receipt_failures(self):
        diag = self.implementation()
        harness = diag.load_harness(REPO)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            originals, reports = [], []
            sha = harness.sha

            def fail_prepare(repo, work, destination, report, shared):
                reports.append(report)
                (destination / "raw.txt").write_bytes(b"preserved diagnostic evidence")
                try:
                    (work / "does-not-exist").read_bytes()
                except FileNotFoundError as error:
                    originals.append(error)
                    raise

            def fail_hash(path):
                if path.name == "raw.txt":
                    raise OSError("secondary artifact hash")
                return sha(path)

            with mock.patch.object(sys, "argv", ["diagnose", "--prepare-only", "--output-dir", str(output)]), \
                    mock.patch.object(diag, "load_harness", return_value=harness), \
                    mock.patch.object(diag, "prepare", side_effect=fail_prepare), \
                    mock.patch.object(harness, "sha", side_effect=fail_hash), \
                    mock.patch.object(harness, "save", side_effect=OSError("secondary receipt")):
                with self.assertRaises(FileNotFoundError) as caught:
                    diag.main()
            self.assertIs(caught.exception, originals[0])
            self.assertEqual(reports[0]["status"], "failed")
            self.assertEqual([item["phase"] for item in reports[0]["secondary_errors"]],
                             ["raw.txt_hash", "receipt_save"])


    def test_native_first_and_actual_returned_category_remain_separate(self):
        diag = self.implementation()
        # 真正 Rust post-check 时序另由 Linux private contracts 验收；这里仅核闭合解析不洗掉字段。
        for returned in ("business_budget_exceeded", "business_permission_denied", "store", "io",
                         "poisoned", "unwind", "unknown"):
            value = self.record(native_failure="conflict", operation="identity", errno=None,
                                dev_equal=True, ino_equal=False, mount_equal=True,
                                returned_category=returned)
            with self.subTest(returned=returned):
                self.assertEqual(diag.collect_causes(self.line(value)), [value])
        invalid = self.record()
        del invalid["returned_category"]
        with self.assertRaises(ValueError):
            diag.collect_causes(self.line(invalid))

    def test_duplicate_json_keys_are_rejected_instead_of_last_value_winning(self):
        diag = self.implementation()
        raw = self.line(self.record()).rstrip(b"\n")
        ambiguous = raw[:-1] + b',"errno":13}\n'
        with self.assertRaises(ValueError):
            diag.collect_causes(ambiguous)

    def test_linux_private_runtime_contracts_are_required_before_measurement(self):
        diag = self.implementation()
        harness = diag.load_harness(REPO)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, work, output = (root / name for name in ("source", "work", "output"))
            for path in (source, work, output):
                path.mkdir()
            report = {"commands": []}
            calls = []

            def recorded_command(argv, cwd, destination, label, receipt, shared, *, cap, timeout, env=None):
                calls.append(list(argv))
                # 只有编排负控；0 cases 不得被认为已经验证 Rust Drop 或 native errno。
                (destination / f"{label}.stdout").write_text(
                    "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n")
                return {"label": label, "exit_code": 0}

            with mock.patch.object(diag, "run_command", side_effect=recorded_command):
                with self.assertRaisesRegex(ValueError, "private.*contracts"):
                    diag.private_contracts(source, work, output, report, harness)
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0][:7], ["cargo", "test", "--release", "--locked", "-p", "diskgraph-engine", "--lib"])
            self.assertIn("native_process::scan_validation_493_diagnostic::tests::", calls[0])

    def test_execute_original_exit101_survives_cause_and_fixture_secondary_failures(self):
        diag = self.implementation()
        harness = diag.load_harness(REPO)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, work, output = (root / name for name in ("source", "work", "output"))
            for path in (source, work, output):
                path.mkdir()
            report = {"commands": []}
            measured = []

            def recorded_command(argv, cwd, destination, label, receipt, shared, *, cap, timeout, env=None):
                code = 0
                if argv[0] == "cargo":
                    binary = Path(env["CARGO_TARGET_DIR"]) / "release/deps/hardening_benchmark"
                    binary.parent.mkdir(parents=True)
                    binary.write_bytes(b"orchestration-only-not-an-ELF")
                    (destination / f"{label}.stdout").write_text(json.dumps({
                        "reason": "compiler-artifact", "target": {"name": "hardening_benchmark"},
                        "executable": str(binary)}) + "\n")
                elif "measure_isolated_release_fixtures" in argv:
                    measured.append(list(argv))
                    code = 101
                    (destination / f"{label}.stderr").write_bytes(PREFIX.encode() + b"not-json\n")
                return {"label": label, "exit_code": code}

            def fixture(root, count, shape):
                root.mkdir()
                return {"orchestration_only": True}

            with mock.patch.object(diag, "run_command", side_effect=recorded_command), \
                    mock.patch.object(harness, "fixture", side_effect=fixture), \
                    mock.patch.object(harness, "verify_fixture", side_effect=OSError("secondary fixture read")):
                with self.assertRaisesRegex(RuntimeError, "exited 101; no retry"):
                    diag.execute(source, work, output, report, harness)
            self.assertEqual(len(measured), 1)
            self.assertEqual(report["measurement_exit_code"], 101)
            self.assertEqual([item["phase"] for item in report["secondary_errors"]],
                             ["cause_parse", "fixture_verification"])
            self.assertFalse(report["cause_observed"])



if __name__ == "__main__":
    unittest.main()
