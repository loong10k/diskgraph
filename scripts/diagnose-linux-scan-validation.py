#!/usr/bin/env python3
"""固定 ae 一次 Linux namespace 验证首错诊断；不修复算法、不重试、不计入 ABBA。"""
import argparse
import difflib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import tempfile

BASELINE = "aeab8a3f2246cb12af74d17d8124ff1076d0e376"
PREFIX = "DG_SCAN_VALIDATION_FIRST_FAILURE "
TEMPLATE = "crates/diskgraph-engine/integration_candidates/scan_validation_diagnostic.rs"
TARGET = "crates/diskgraph-engine/src/native_process/scan_validation_diagnostic.rs"
TEMPLATE_SHA256 = "5227da77540f58f481ad26951e86c065775840721d566e89c6dfe22eec0dead0"
SOURCE_HASHES = {
    "crates/diskgraph-engine/src/scan_execution.rs": "92ef03e07dc19c8920258e6e9cd599ae007c112df02cc44c999ab559775ce2b2",
    "crates/diskgraph-engine/src/native_process/mod.rs": "92b2bded52bd9e54998e3808122daf7883cdc619e3404656c07a29f11576b289",
    "crates/diskgraph-engine/src/native_process/linux_scan_root.rs": "a0424783cb863ee8097dc4074516c7004da6170e6fc87be3fce8b9ca6bcdfee9",
    "crates/diskgraph-engine/src/native_process/linux_scan_namespace.rs": "86105befe2f907279f87b4f617a977fb1e70e47cdb03cc7739828e84d8f3b6ab",
    "crates/diskgraph-engine/src/native_process/linux_directory_identity.rs": "ebdcd42f2e9b6d90ffa3edb8c6306a931bd7e96460d752cdeb0f9923294f7c2a",
    "crates/diskgraph-engine/src/native_process/linux_open.rs": "b569eda8885214eb66f9fc026daa9751e504f88ac6cb7f347a5f2e0441d635d8",
}
HARNESS_FILES = (
    "crates/diskgraph-engine/tests/hardening_benchmark.rs",
    "crates/diskgraph-engine/tests/benchmark_support/mod.rs",
    "crates/diskgraph-engine/tests/benchmark_support/namespace_cost.rs",
    "crates/diskgraph-engine/tests/benchmark_support/scan_failure_diagnostic.rs",
)


def load_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# 复用已测主/次错误及真实进程收尾，不替换旧 2a 常量、patch 或执行方法。
TRANSPORT = load_module(Path(__file__).with_name("diagnose-linux-baseline.py"), "old_baseline_transport")
run_command = TRANSPORT.run_command
finalize_report = TRANSPORT.finalize_report
finalize_step = TRANSPORT.finalize_step
require_success = TRANSPORT.require_success
load_harness = TRANSPORT.load_harness


def digest(body):
    return hashlib.sha256(body).hexdigest()


def replacements(name):
    """仅返回固定原文件的可逆观察插入，不更改原权限、时钟、解析 flags 或映射分支。"""
    diagnostic = "super::scan_validation_diagnostic::ScanValidationDiagnostic"
    if name not in SOURCE_HASHES:
        raise ValueError("unapproved diagnostic source")
    if name.endswith("/scan_execution.rs"):
        before = "        let options = self.scan_options.clone();\n"
        after = ("        #[cfg(target_os = \"linux\")]\n"
                 "        let _validation_execution = crate::native_process::scan_validation_diagnostic::ScanValidationDiagnostic::execution(\n"
                 "            scan_started, job.owner == owner && owner == \"benchmark\",\n"
                 "            job.principal.as_str() == \"benchmark\",\n"
                 "        );\n" + before)
        return [(before, after)]
    if name.endswith("/mod.rs"):
        before = "mod process_native_session;\n"
        return [(before, before + "#[cfg(target_os = \"linux\")]\npub(crate) mod scan_validation_diagnostic;\n")]
    if name.endswith("/linux_scan_root.rs"):
        before = ("        let result = checked_native(check, &|native_check| self.namespace.verify(native_check))?;\n"
                  "        check()?;\n")
        return [(before, f"        let diagnostic = {diagnostic}::begin();\n" + before
                 + "        diagnostic.finish(&result);\n")]
    if name.endswith("/linux_scan_namespace.rs"):
        return [
            ("        let current_anchor = open_at(\n",
             f"        {diagnostic}::component(0);\n        let current_anchor = open_at(\n"),
            ("        for (name, original) in &self.route {\n",
             "        for (index, (name, original)) in self.route.iter().enumerate() {\n"
             f"            {diagnostic}::component(index + 1);\n"),
        ]
    if name.endswith("/linux_directory_identity.rs"):
        return [
            ("        let metadata = current.metadata().map_err(|_| Failure::Unavailable)?;\n",
             "        let metadata = current.metadata().map_err(|error| {\n"
             f"            {diagnostic}::native(3, error.raw_os_error());\n"
             "            Failure::Unavailable\n        })?;\n"),
            ("        if self.device != metadata.dev() || self.inode != metadata.ino() || self.mount != mount {\n",
             "        if self.device != metadata.dev() || self.inode != metadata.ino() || self.mount != mount {\n"
             f"            {diagnostic}::identity(self.device == metadata.dev(), self.inode == metadata.ino(), self.mount == mount);\n"),
        ]
    return [
        ("    if fd < 0 {\n        return Err(last_error());\n    }\n",
         "    if fd < 0 {\n        let error = std::io::Error::last_os_error();\n"
         f"        {diagnostic}::native(1, error.raw_os_error());\n"
         "        return Err(classify_error(error.raw_os_error()));\n    }\n"),
        ("    } != 0\n    {\n        return Err(last_error());\n    }\n",
         "    } != 0\n    {\n        let error = std::io::Error::last_os_error();\n"
         f"        {diagnostic}::native(2, error.raw_os_error());\n"
         "        return Err(classify_error(error.raw_os_error()));\n    }\n"),
        ("    if value.stx_mask & UNIQUE == 0 || value.stx_mnt_id == 0 {\n",
         "    if value.stx_mask & UNIQUE == 0 || value.stx_mnt_id == 0 {\n"
         f"        {diagnostic}::native(2, None);\n"),
        ("    match std::io::Error::last_os_error().raw_os_error() {\n",
         "    classify_error(std::io::Error::last_os_error().raw_os_error())\n}\n\n"
         "fn classify_error(raw_os_error: Option<i32>) -> Failure {\n    match raw_os_error {\n"),
    ]


def patch_source(name, original):
    if name not in SOURCE_HASHES or digest(original) != SOURCE_HASHES[name]:
        raise ValueError(f"immutable diagnostic source hash mismatch: {name}")
    before = original.decode("utf-8")
    after = before
    changes = replacements(name)
    for old, new in changes:
        if after.count(old) != 1:
            raise ValueError(f"diagnostic anchor must occur exactly once: {name}")
        after = after.replace(old, new, 1)
    restored = after
    for old, new in reversed(changes):
        if restored.count(new) != 1:
            raise ValueError("diagnostic inverse anchor mismatch")
        restored = restored.replace(new, old, 1)
    if restored != before:
        raise ValueError("non-diagnostic source mutation")
    return after.encode(), "".join(difflib.unified_diff(
        before.splitlines(keepends=True), after.splitlines(keepends=True),
        fromfile=f"original/{name}", tofile=f"diagnostic-only/{name}"))


def collect_causes(raw):
    """只收本次原验证失败的闭合标量；后验 JSON、未知文本和重复首因不能替代此见证。"""
    fields = {"phase", "operation", "native_failure", "errno", "component", "elapsed_us",
              "validation", "owner_matches", "principal_matches", "dev_equal", "ino_equal", "mount_equal"}
    result = []
    for line in raw.splitlines():
        if not line.startswith(PREFIX.encode()):
            continue
        if result or len(line) > 2048:
            raise ValueError("duplicate or oversized first-cause record")
        try:
            record = json.loads(line[len(PREFIX):])
        except (ValueError, UnicodeError) as error:
            raise ValueError("invalid first-cause JSON") from error
        if not isinstance(record, dict) or set(record) != fields:
            raise ValueError("unapproved first-cause fields")
        if record["phase"] != "namespace_validate" or record["operation"] not in ("openat2", "statx", "fstat", "identity", "unknown"):
            raise ValueError("unapproved diagnostic phase or operation")
        if record["native_failure"] not in ("budget_exceeded", "timeout", "cancelled", "permission_denied",
                                           "conflict", "unsupported", "unavailable", "internal_error"):
            raise ValueError("unapproved native failure")
        for name in ("errno", "component"):
            if record[name] is not None and (type(record[name]) is not int or record[name] < 0):
                raise ValueError("non-scalar errno or component")
        for name, minimum in (("elapsed_us", 0), ("validation", 1)):
            if type(record[name]) is not int or record[name] < minimum:
                raise ValueError("invalid diagnostic counter")
        for name in ("owner_matches", "principal_matches"):
            if type(record[name]) is not bool:
                raise ValueError("invalid request binding boolean")
        for name in ("dev_equal", "ino_equal", "mount_equal"):
            if record[name] is not None and type(record[name]) is not bool:
                raise ValueError("invalid identity comparison boolean")
        result.append(record)
    return result


def prepare(repo, work, output, report, harness):
    import tarfile
    archive = run_command(["git", "archive", "--format=tar", BASELINE], repo, output,
                          "baseline-archive", report, harness, cap=TRANSPORT.ARCHIVE_CAP, timeout=120)
    require_success(archive)
    source = work / "diagnostic-source"
    source.mkdir()
    with tarfile.open(output / "baseline-archive.stdout") as bundle:
        bundle.extractall(source, filter="data")
    original = harness.inventory(source)
    harness.save(output / "original-source.json", original)
    overlay = {}
    for name in HARNESS_FILES:
        body = (repo / name).read_bytes()
        for destination in (source / name, output / "harness" / name):
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(body)
        overlay[name] = digest(body)
    harness.save(output / "harness-source.json", harness.inventory(source))
    patches = []
    for name in SOURCE_HASHES:
        before = (source / name).read_bytes()
        after, difference = patch_source(name, before)
        for prefix, body in (("before", before), ("after", after)):
            destination = output / prefix / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(body)
        (source / name).write_bytes(after)
        patches.append(difference)
    template = (repo / TEMPLATE).read_bytes()
    if digest(template) != TEMPLATE_SHA256 or (source / TARGET).exists():
        raise ValueError("private diagnostic template mismatch or target already exists")
    (source / TARGET).write_bytes(template)
    destination = output / "after" / TARGET
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(template)
    patches.append("".join(difflib.unified_diff([], template.decode().splitlines(keepends=True),
                                               fromfile="/dev/null", tofile=f"diagnostic-only/{TARGET}")))
    (output / "diagnostic-only.diff").write_text("".join(patches))
    measured = harness.inventory(source)
    harness.save(output / "diagnostic-source.json", measured)
    changed = {name for name in original.keys() | measured.keys() if original.get(name) != measured.get(name)}
    expected = set(HARNESS_FILES) | set(SOURCE_HASHES) | {TARGET}
    if not changed <= expected:
        raise ValueError(f"unexpected source overlay: {sorted(changed - expected)}")
    report["source"] = {"commit": BASELINE, "archive_sha256": archive["stdout"]["sha256"],
                        "harness": overlay, "diagnostic_before": SOURCE_HASHES,
                        "diagnostic_after": {name: measured[name] for name in SOURCE_HASHES},
                        "template_sha256": TEMPLATE_SHA256, "actual_changed_files": sorted(changed),
                        "lock_sha256": harness.sha(source / "Cargo.lock")}
    harness.save(output / "receipt.json", report)
    return source


def native_environment(work):
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(work / "target-diagnostic-only")
    env["RUSTFLAGS"] = env.get("RUSTFLAGS", "") + " -Dwarnings"
    if env.get("CARGO_ENCODED_RUSTFLAGS") or env.get("CARGO_BUILD_TARGET"):
        raise ValueError("encoded flags/cross target override cannot qualify this native experiment")
    return env


def private_contracts(source, work, output, report, harness):
    env = native_environment(work)
    unit = run_command(["cargo", "test", "--release", "--locked", "-p", "diskgraph-engine", "--lib",
                        "native_process::scan_validation_diagnostic::tests::", "--", "--nocapture", "--test-threads=1"],
                       source, output, "diagnostic-private-contracts", report, harness,
                       cap=TRANSPORT.BUILD_CAP, timeout=1800, env=env)
    require_success(unit)
    if not re.search(rb"test result: ok\. 4 passed; 0 failed; 0 ignored;",
                     (output / "diagnostic-private-contracts.stdout").read_bytes()):
        raise ValueError("all four private Linux diagnostic contracts must actually execute")


def execute(source, work, output, report, harness):
    env = native_environment(work)
    build = run_command(["cargo", "test", "--release", "--locked", "-p", "diskgraph-engine",
                         "--test", "hardening_benchmark", "--no-run", "--message-format=json"],
                        source, output, "diagnostic-only-build", report, harness,
                        cap=TRANSPORT.BUILD_CAP, timeout=1800, env=env)
    require_success(build)
    binaries = []
    for line in (output / "diagnostic-only-build.stdout").read_text().splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        if entry.get("reason") == "compiler-artifact" and entry.get("target", {}).get("name") == "hardening_benchmark" and entry.get("executable"):
            binaries.append(Path(entry["executable"]))
    if len(binaries) != 1:
        raise ValueError("expected exactly one fresh benchmark executable")
    binary = binaries[0].resolve()
    if not binary.is_file() or not binary.is_relative_to(Path(env["CARGO_TARGET_DIR"]).resolve()):
        raise ValueError("compiler artifact is outside fresh diagnostic target")
    report["binary"] = {"path": str(binary), "sha256": harness.sha(binary)}
    root, data = work / "fixture-200000-wide", work / "fresh-data"
    report["fixture"] = harness.fixture(root, 200000, "wide")
    require_success(run_command(["stat", "-f", "-c", "%T %s %b %f", str(root)], work, output,
                                "filesystem", report, harness, cap=TRANSPORT.RUN_CAP, timeout=30))
    if data.exists():
        raise ValueError("diagnostic data directory must be fresh")
    metrics = output / "diagnostic-only-metrics.json"
    env.update(DG_MEASURE_CASE="200000", DG_MEASURE_SHAPE="wide", DG_MEASURE_ROOT=str(root),
               DG_MEASURE_DATA=str(data), DISKGRAPH_BENCHMARK_OUTPUT=str(metrics))
    if harness.sha(binary) != report["binary"]["sha256"]:
        raise ValueError("fresh diagnostic executable changed")
    result = run_command([str(binary), "--exact", "measure_isolated_release_fixtures", "--ignored",
                          "--nocapture", "--test-threads=1"], work, output,
                         "diagnostic-only-200000-wide", report, harness,
                         cap=TRANSPORT.RUN_CAP, timeout=1200, env=env)
    report["measurement_exit_code"] = result["exit_code"]
    # 解析/后验校验属于 secondary；不能遮盖本次真实 Rust 非零退出。
    errors = []
    finalize_step(report, errors, "cause_parse", lambda: report.update(
        causes=collect_causes((output / "diagnostic-only-200000-wide.stderr").read_bytes())))
    report["cause_observed"] = bool(report.get("causes"))
    finalize_step(report, errors, "fixture_verification", lambda: harness.verify_fixture(root, report["fixture"]))
    require_success(result)
    if errors:
        raise errors[0]
    parsed = json.loads(metrics.read_text())
    if (len(parsed) != 1 or parsed[0]["files"] != 200000 or parsed[0]["shape"] != "wide"
            or parsed[0]["native_qualification"]["qualified"] is not True):
        raise ValueError("successful diagnostic scan lacks native qualification")
    report["metrics_sha256"] = harness.sha(metrics)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--prepare-only", action="store_true")
    args = parser.parse_args()
    if not args.prepare_only and platform.system() != "Linux":
        parser.error("actual diagnostic requires native Linux")
    repo = Path(__file__).resolve().parents[1]
    harness = load_harness(repo)
    output = args.output_dir.resolve()
    if output == repo or output.is_relative_to(repo):
        parser.error("diagnostic output must be outside the repository")
    output.mkdir(parents=True, exist_ok=False)
    report = {"schema_version": 1, "kind": "diagnostic-only", "status": "running", "commands": [],
              "baseline": BASELINE, "planned_measurements": 0 if args.prepare_only else 1,
              "script_sha256": harness.sha(Path(__file__)),
              "transport_sha256": harness.sha(Path(TRANSPORT.__file__)),
              "harness_script_sha256": harness.sha(repo / "scripts/accept-linux-scan-namespace-cost.py"),
              "hardware": {"uname": list(platform.uname()), "cpu_count": os.cpu_count()},
              "limitations": ["Not ABBA data; no retry or production/cache policy change.",
                              "Success or no cause record does not explain the previous failure.",
                              "Immediate errno and identity booleans only cover actual namespace validate calls.",
                              "Logging/instrumentation can affect scheduling; raw capture caps are not process RSS bounds.",
                              "Post-cleanup empty staging and recent heartbeat are not first-cause witnesses."]}
    primary, temporary = None, None
    try:
        temporary = tempfile.TemporaryDirectory(prefix="diskgraph-ae-validation-", dir=os.environ.get("RUNNER_TEMP"))
        work = Path(temporary.name)
        source = prepare(repo, work, output, report, harness)
        if not args.prepare_only:
            for command in (["rustc", "-vV"], ["cargo", "--version"]):
                require_success(run_command(command, repo, output, command[0], report, harness,
                                            cap=TRANSPORT.RUN_CAP, timeout=30))
            private_contracts(source, work, output, report, harness)
            execute(source, work, output, report, harness)
        report["status"] = "prepared_only" if args.prepare_only else "diagnostic_completed"
    except BaseException as error:
        primary = error
        report.update(status="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        errors = []
        if temporary is not None:
            finalize_step(report, errors, "temporary_cleanup", temporary.cleanup)
        cleanup_primary = errors[0] if errors and primary is None else None
        if cleanup_primary is not None:
            report.update(status="failed", error=f"{type(cleanup_primary).__name__}: {cleanup_primary}")
        finalize_report(output, report, harness, primary if primary is not None else cleanup_primary)
        if cleanup_primary is not None:
            raise cleanup_primary


if __name__ == "__main__":
    main()
