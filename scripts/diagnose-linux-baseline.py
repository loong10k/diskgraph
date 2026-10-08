#!/usr/bin/env python3
"""仅一次旧 2a Linux 扫描首因诊断；不属于 ABBA 性能统计、不重试、不修复旧代码。"""
import argparse
import difflib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import selectors
import shutil
import signal
import subprocess
import tarfile
import tempfile
import time

BASELINE = "2a2f8281f9f211b6632bdb26afdcd4a4fb21a44d"
PREFIX = "diskgraph-baseline-diagnostic phase="
SOURCE_HASHES = {
    "crates/diskgraph-engine/src/scan_jobs.rs": "53a4d5b03528b365d635651f15bbacaaa796fd12dcff892cef60e1ffddd8e826",
    "crates/diskgraph-engine/src/scan_execution.rs": "38ff45b940bf28ec9594aa36f84b104c21c144afce974ff4dfa4268de43222ff",
}
ARCHIVE_CAP = 256 * 1024 * 1024
BUILD_CAP = 64 * 1024 * 1024
RUN_CAP = 4 * 1024 * 1024
HARNESS_FILES = (
    "crates/diskgraph-engine/tests/hardening_benchmark.rs",
    "crates/diskgraph-engine/tests/benchmark_support/mod.rs",
    "crates/diskgraph-engine/tests/benchmark_support/peak_memory.rs",
    "crates/diskgraph-engine/tests/benchmark_support/benchmark_engine.rs",
    "crates/diskgraph-engine/tests/benchmark_support/namespace_cost.rs",
    "crates/diskgraph-engine/tests/benchmark_support/scan_failure_diagnostic.rs",
)


def load_harness(repo):
    path = repo / "scripts/accept-linux-scan-namespace-cost.py"
    spec = importlib.util.spec_from_file_location("namespace_cost_harness", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if tuple(module.HARNESS) != HARNESS_FILES:
        raise ValueError("expected exactly the six approved benchmark harness files")
    return module


def digest(body):
    return hashlib.sha256(body).hexdigest()


def emit(phase, expression, indent):
    # 不 format! 复制错误，不 unwrap 写入；stderr I/O 失败不得替换原业务结果。
    return (f'{indent}let _ = std::io::Write::write_fmt(\n'
            f'{indent}    &mut std::io::stderr().lock(),\n'
            f'{indent}    format_args!("{PREFIX}{phase} elapsed_us={{}} error={{:?}}\\n", scan_started.elapsed().as_micros(), {expression}),\n'
            f'{indent});\n')


def replacements(name):
    if name.endswith("scan_jobs.rs"):
        return [
            ("                    if checked.is_err() {\n",
             "                    if let Err(ref error) = checked {\n" + emit("keeper_checked", "error", "                        ")),
            ("            let _ = keeper.join();\n",
             "            let diagnostic_join = keeper.join();\n"
             "            if let Err(ref payload) = diagnostic_join {\n"
             "                let diagnostic_payload = payload.downcast_ref::<String>().map(String::as_str)\n"
             "                    .or_else(|| payload.downcast_ref::<&str>().copied());\n"
             + emit("keeper_join_panic", "diagnostic_payload", "                ")
             + "            }\n"),
        ]
    return [
        ("        let mut exceeded = false;\n", "        let mut exceeded = false;\n        let mut diagnostic_guard_seen = false;\n"),
        ("                if self\n                    .control()?\n                    .heartbeat_fenced(job_id, owner, job.fencing_token)\n                    .is_err()\n                {\n",
         "                if let Err(error) = self\n                    .control()?\n                    .heartbeat_fenced(job_id, owner, job.fencing_token)\n                {\n" + emit("walk_heartbeat", "error", "                    ")),
        ("            if observation_guard.check_now().is_err() {\n",
         "            if let Err(error) = observation_guard.check_now() {\n"
         "                if !diagnostic_guard_seen {\n"
         + emit("walk_observation_guard", "error", "                    ")
         + "                    diagnostic_guard_seen = true;\n                }\n"),
    ]


def patch_source(name, original):
    if digest(original) != SOURCE_HASHES[name]:
        raise ValueError(f"immutable baseline source hash mismatch: {name}")
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


def kill_owned_group(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def finalize_step(record, errors, phase, work):
    """逐项保留收尾错误；不在已有 primary 的栈展开中抛出替代错误。"""
    try:
        work()
    except BaseException as error:
        errors.append(error)
        record.setdefault("secondary_errors", []).append(
            {"phase": phase, "type": type(error).__name__, "message": str(error)})


def metadata(path, destination, record, errors, phase, harness):
    destination["path"] = path.name
    finalize_step(record, errors, f"{phase}_stat", lambda: destination.update(bytes=path.stat().st_size))
    finalize_step(record, errors, f"{phase}_hash", lambda: destination.update(sha256=harness.sha(path)))


def finalize_report(output, report, harness, primary):
    """归档各项独立失败；回执本身不可写时保留内存记录并维持原异常。"""
    errors, paths = [], []
    report["artifacts"] = {}
    finalize_step(report, errors, "archive_enumeration", lambda: paths.extend(sorted(output.rglob("*"))))
    for path in paths:
        if path.name == "receipt.json":
            continue
        files = []
        finalize_step(report, errors, "archive_type", lambda: files.append(path.is_file()))
        if files == [True]:
            entry = {}
            report["artifacts"][str(path.relative_to(output))] = entry
            metadata(path, entry, report, errors, str(path.relative_to(output)), harness)
    if errors and primary is None:
        report.update(status="failed", error=f"{type(errors[0]).__name__}: {errors[0]}")
    finalize_step(report, errors, "receipt_save", lambda: harness.save(output / "receipt.json", report))
    if errors and primary is None:
        report["status"] = "failed"
        raise errors[0]


def run_command(argv, cwd, output, label, report, harness, *, cap, timeout, env=None):
    """公平排空两个管道，各自硬上限；失败保留原始前缀及整组终止事实。"""
    started = time.monotonic()
    record = {"label": label, "argv": [str(arg) for arg in argv], "cwd": str(cwd),
              "timeout_seconds": timeout, "per_stream_cap_bytes": cap,
              "environment": {key: (env or os.environ).get(key) for key in harness.ENV_KEYS}}
    report["commands"].append(record)
    process = None
    selector = None
    primary = None
    files = {}
    try:
        selector = selectors.DefaultSelector()
        for stream in ("stdout", "stderr"):
            files[stream] = (output / f"{label}.{stream}").open("wb")
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        record["process_group_id"] = process.pid
        counts = {"stdout": 0, "stderr": 0}
        for name in counts:
            pipe = getattr(process, name)
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, name)
        while selector.get_map() or process.poll() is None:
            remaining = timeout - (time.monotonic() - started)
            if remaining <= 0:
                raise TimeoutError(f"{label}: original command deadline expired")
            for key, _ in selector.select(min(remaining, 0.1)):
                data = os.read(key.fd, 65536)
                if not data:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                name = key.data
                allowed = min(len(data), cap - counts[name])
                files[name].write(data[:allowed])
                counts[name] += allowed
                if allowed != len(data):
                    record["truncated_stream"] = name
                    raise OverflowError(f"{label}: {name} raw capture cap exceeded")
        record["exit_code"] = process.wait()
    except BaseException as error:
        primary = error
        record.update(error_type=type(error).__name__, error=str(error))
        if process is not None:
            try:
                kill_owned_group(process)
                record["terminated_process_group"] = True
                record["exit_code"] = process.returncode
            except BaseException as cleanup:
                record["cleanup_error"] = f"{type(cleanup).__name__}: {cleanup}"
        raise
    finally:
        errors = []
        for name, file in files.items():
            finalize_step(record, errors, f"{name}_close", file.close)
        if process is not None:
            for name in ("stdout", "stderr"):
                pipe = getattr(process, name)
                if pipe is not None:
                    finalize_step(record, errors, f"{name}_pipe_close", pipe.close)
        if selector is not None:
            finalize_step(record, errors, "selector_close", selector.close)
        record["elapsed_seconds"] = time.monotonic() - started
        for name in files:
            record[name] = {}
            metadata(output / f"{label}.{name}", record[name], record, errors, name, harness)
        finalize_step(record, errors, "receipt_save", lambda: harness.save(output / "receipt.json", report))
        # 非零真实子进程退出也是已知 primary，由原 require_success 分类，不让回执失败替换它。
        if errors and primary is None and record.get("exit_code", 0) == 0:
            raise errors[0]
    return record


def require_success(record):
    if record["exit_code"] != 0:
        raise RuntimeError(f"{record['label']} exited {record['exit_code']}; no retry")


def prepare(repo, work, output, report, harness):
    archive = run_command(["git", "archive", "--format=tar", BASELINE], repo, output,
                          "baseline-archive", report, harness, cap=ARCHIVE_CAP, timeout=120)
    require_success(archive)
    source = work / "diagnostic-source"
    source.mkdir()
    with tarfile.open(output / "baseline-archive.stdout") as bundle:
        bundle.extractall(source, filter="data")
    original = harness.inventory(source)
    harness.save(output / "original-source.json", original)
    overlay = {}
    for name in harness.HARNESS:
        body = harness.LEGACY_ADAPTER.encode() if name == harness.ADAPTER else (repo / name).read_bytes()
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
    (output / "diagnostic-only.diff").write_text("".join(patches))
    measured = harness.inventory(source)
    harness.save(output / "diagnostic-source.json", measured)
    expected = set(harness.HARNESS) | set(SOURCE_HASHES)
    changed = {name for name in set(original) | set(measured) if original.get(name) != measured.get(name)}
    if not changed <= expected:
        raise ValueError(f"unexpected source overlay: {sorted(changed - expected)}")
    report["source"] = {"commit": BASELINE, "archive_sha256": archive["stdout"]["sha256"],
                        "harness": overlay, "harness_original": {name: original.get(name) for name in HARNESS_FILES},
                        "diagnostic_before": SOURCE_HASHES,
                        "diagnostic_after": {name: measured[name] for name in SOURCE_HASHES},
                        "actual_changed_files": sorted(changed), "lock_sha256": harness.sha(source / "Cargo.lock")}
    harness.save(output / "receipt.json", report)
    return source


def execute(source, work, output, report, harness):
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(work / "target-diagnostic-only")
    build = run_command(["cargo", "test", "--release", "--locked", "-p", "diskgraph-engine",
                         "--test", "hardening_benchmark", "--no-run", "--message-format=json"],
                        source, output, "diagnostic-only-build", report, harness,
                        cap=BUILD_CAP, timeout=1800, env=env)
    require_success(build)
    binaries = []
    with (output / "diagnostic-only-build.stdout").open() as stream:
        for line in stream:
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (entry.get("reason") == "compiler-artifact"
                    and entry.get("target", {}).get("name") == "hardening_benchmark"
                    and entry.get("executable")):
                binaries.append(Path(entry["executable"]))
    if len(binaries) != 1:
        raise ValueError("expected exactly one fresh benchmark executable")
    binary = binaries[0].resolve()
    if not binary.is_file() or not binary.is_relative_to((work / "target-diagnostic-only").resolve()):
        raise ValueError("compiler artifact is outside the fresh diagnostic target")
    report["binary"] = {"path": str(binary), "sha256": harness.sha(binary)}
    root, data = work / "fixture-200000-wide", work / "fresh-data"
    report["fixture"] = harness.fixture(root, 200000, "wide")
    require_success(run_command(["stat", "-f", "-c", "%T %s %b %f", str(root)], work, output,
                                "filesystem", report, harness, cap=RUN_CAP, timeout=30))
    if data.exists():
        raise ValueError("diagnostic data directory must be fresh")
    metrics = output / "diagnostic-only-metrics.json"
    env.update(DG_MEASURE_CASE="200000", DG_MEASURE_SHAPE="wide", DG_MEASURE_ROOT=str(root),
               DG_MEASURE_DATA=str(data), DISKGRAPH_BENCHMARK_OUTPUT=str(metrics))
    if harness.sha(binary) != report["binary"]["sha256"]:
        raise ValueError("fresh diagnostic executable changed before measurement")
    result = run_command([str(binary), "--exact", "measure_isolated_release_fixtures", "--ignored",
                          "--nocapture", "--test-threads=1"], work, output,
                         "diagnostic-only-200000-wide", report, harness,
                         cap=RUN_CAP, timeout=1200, env=env)
    report["measurement_exit_code"] = result["exit_code"]
    stderr = (output / "diagnostic-only-200000-wide.stderr").read_text(errors="replace")
    report["cause_lines"] = [line for line in stderr.splitlines() if PREFIX in line]
    report["cause_observed"] = bool(report["cause_lines"])
    try:
        harness.verify_fixture(root, report["fixture"])
        report["fixture"]["verified_unchanged_after_single_run"] = True
    except BaseException as error:
        report["fixture_verification_error"] = f"{type(error).__name__}: {error}"
        if result["exit_code"] == 0:
            raise
    require_success(result)
    parsed = json.loads(metrics.read_text())
    if (len(parsed) != 1 or parsed[0]["files"] != 200000 or parsed[0]["shape"] != "wide"
            or parsed[0]["native_qualification"]["qualified"] is not True):
        raise ValueError("diagnostic successful scan lacks native qualification")
    report["metrics_sha256"] = harness.sha(metrics)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--prepare-only", action="store_true", help="只读 Git 导出与精确 patch，不编译或测量")
    args = parser.parse_args()
    if not args.prepare_only and platform.system() != "Linux":
        parser.error("actual diagnostic requires native Linux")
    repo = Path(__file__).resolve().parents[1]
    harness = load_harness(repo)
    output = args.output_dir.resolve()
    if output == repo or output.is_relative_to(repo):
        parser.error("diagnostic artifacts must be outside the source repository")
    output.mkdir(parents=True, exist_ok=False)
    report = {"schema_version": 1, "kind": "diagnostic-only", "status": "running", "commands": [],
              "baseline": BASELINE, "planned_measurements": 0 if args.prepare_only else 1,
              "script_sha256": harness.sha(Path(__file__)),
              "harness_script_sha256": harness.sha(repo / "scripts/accept-linux-scan-namespace-cost.py"),
              "hardware": {"uname": list(platform.uname()), "cpu_count": os.cpu_count()},
              "limitations": ["Never part of the twelve ABBA measurements or performance ratios.",
                              "Logging can change scheduling; success does not fix the baseline failure.",
                              "Only four observed error-loss sites; no event means cause not observed.",
                              "Logs are raw bounded prefixes; overflow fails and terminates the owned group."]}
    primary, temporary = None, None
    try:
        temporary = tempfile.TemporaryDirectory(prefix="diskgraph-baseline-diagnostic-", dir=os.environ.get("RUNNER_TEMP"))
        work = Path(temporary.name)
        source = prepare(repo, work, output, report, harness)
        if not args.prepare_only:
            for command in (["rustc", "-vV"], ["cargo", "--version"]):
                require_success(run_command(command, repo, output, command[0], report, harness,
                                            cap=RUN_CAP, timeout=30))
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
