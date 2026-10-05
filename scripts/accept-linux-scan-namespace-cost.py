#!/usr/bin/env python3
"""固定 Linux 提交配对成本验收；只操作自建临时目录，不改变工作树或系统缓存。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import tarfile
import tempfile
import time

BASE = "5c9985b84645dcac8c82ae06903ee7249b06dbdd"
NEW = "2a2f8281f9f211b6632bdb26afdcd4a4fb21a44d"
HARNESS = ["crates/diskgraph-engine/tests/hardening_benchmark.rs",
           "crates/diskgraph-engine/tests/benchmark_support/mod.rs",
           "crates/diskgraph-engine/tests/benchmark_support/namespace_cost.rs",
           "crates/diskgraph-engine/tests/benchmark_support/scan_failure_diagnostic.rs"]
ENV_KEYS = ["PATH", "RUSTFLAGS", "RUSTUP_TOOLCHAIN", "CARGO_BUILD_JOBS",
            "CARGO_TARGET_DIR", "TMPDIR", "LANG", "LC_ALL", "DG_MEASURE_CASE",
            "DG_MEASURE_SHAPE", "DG_MEASURE_ROOT", "DG_MEASURE_DATA", "DISKGRAPH_BENCHMARK_OUTPUT"]


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def save(path, value):
    temporary = path.with_suffix(".writing")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def inventory(root):
    return {str(path.relative_to(root)): sha(path) for path in sorted(root.rglob("*"))
            if path.is_file() and not path.is_symlink()}


def terminate_group(process):
    """终止自建会话的整组后代并回收直接子进程，避免超时 Cargo 留下编译器。"""
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def command(argv, cwd, output, label, report, env=None, timeout=1200, required=True):
    stdout, stderr = output / f"{label}.stdout", output / f"{label}.stderr"
    started = time.monotonic()
    record = {"argv": [str(arg) for arg in argv], "cwd": str(cwd), "label": label,
              "environment": {key: (env or os.environ).get(key) for key in ENV_KEYS},
              "stdout": stdout.name, "stderr": stderr.name}
    report["commands"].append(record)
    with stdout.open("wb") as out, stderr.open("wb") as err:
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=out, stderr=err,
                                   start_new_session=True)
        record["process_group_id"] = process.pid
        try:
            record["exit_code"] = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            terminate_group(process)
            record["exit_code"] = None
            record["timed_out"] = True
            record["timeout_terminated_process_group"] = True
            record["termination_returncode"] = process.returncode
        except BaseException as error:
            terminate_group(process)
            record.update(exit_code=process.returncode, aborted=True,
                          error=type(error).__name__, terminated_process_group=True,
                          termination_returncode=process.returncode)
            raise
        finally:
            record.update(elapsed_seconds=time.monotonic() - started,
                          stdout_sha256=sha(stdout), stderr_sha256=sha(stderr))
            save(output / "receipt.json", report)
    if required and record["exit_code"] != 0:
        raise RuntimeError(f"{label} failed: {record}")
    return record


def fixture(root, count, shape):
    root.mkdir()
    parent = root
    digest = hashlib.sha256()
    identities = hashlib.sha256()
    for index in range(count):
        if shape == "deep":
            parent = parent / "d"
            parent.mkdir()
        path = parent / f"file-{index:06}"
        path.write_bytes(bytes(32))
        digest.update(str(path.relative_to(root)).encode() + b"\0" + bytes(32))
        stat = path.stat()
        identities.update(f"{index}:{stat.st_dev}:{stat.st_ino}\n".encode())
    state = root.stat()
    return {"files": count, "shape": shape, "root": str(root),
            "root_device": state.st_dev, "root_inode": state.st_ino,
            "root_component_depth": len(root.parts), "content_recipe_sha256": digest.hexdigest(),
            "file_identity_sha256": identities.hexdigest(), "content_bytes_per_file": 32, "source_mutated_during_measurement": False}


def verify_fixture(root, expected):
    state = root.stat()
    assert (state.st_dev, state.st_ino) == (expected["root_device"], expected["root_inode"])
    digest = hashlib.sha256()
    identities = hashlib.sha256()
    parent = root
    for index in range(expected["files"]):
        if expected["shape"] == "deep":
            parent = parent / "d"
        path = parent / f"file-{index:06}"
        body = path.read_bytes()
        assert body == bytes(32), f"fixture body changed: {path}"
        digest.update(str(path.relative_to(root)).encode() + b"\0" + body)
        stat = path.stat()
        identities.update(f"{index}:{stat.st_dev}:{stat.st_ino}\n".encode())
    assert digest.hexdigest() == expected["content_recipe_sha256"]
    assert identities.hexdigest() == expected["file_identity_sha256"]


def build_pair(repo, work, output, report, baseline=BASE, candidate=NEW):
    binaries = {}
    for label, revision in [("baseline", baseline), ("candidate", candidate)]:
        exists = command(["git", "cat-file", "-e", f"{revision}^{{commit}}"], repo,
                         output, f"{label}-object", report, required=False)
        if exists["exit_code"] != 0:
            command(["git", "fetch", "--no-tags", "--depth=1", "origin", revision],
                    repo, output, f"{label}-fetch", report)
        archive = command(["git", "archive", "--format=tar", revision], repo,
                          output, f"{label}-archive", report)
        source = work / label
        source.mkdir()
        with tarfile.open(output / archive["stdout"]) as bundle:
            bundle.extractall(source, filter="data")
        original = inventory(source)
        save(output / f"{label}-original-source.json", original)
        for name in HARNESS:
            destination = source / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(repo / name, destination)
        overlaid = inventory(source)
        save(output / f"{label}-measured-source.json", overlaid)
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(work / f"target-{label}")
        build = command(["cargo", "test", "--release", "--locked", "-p", "diskgraph-engine",
                         "--test", "hardening_benchmark", "--no-run", "--message-format=json"],
                        source, output, f"{label}-build", report, env=env, timeout=1800)
        candidates = []
        for line in (output / build["stdout"]).read_text().splitlines():
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (message.get("reason") == "compiler-artifact"
                    and message.get("target", {}).get("name") == "hardening_benchmark"
                    and message.get("executable")):
                candidates.append(Path(message["executable"]))
        assert len(candidates) == 1, candidates
        binary = candidates[0]
        binaries[label] = binary
        report["sources"][label] = {"commit": revision, "archive_sha256": archive["stdout_sha256"],
            "original_manifest_sha256": sha(output / f"{label}-original-source.json"),
            "measured_manifest_sha256": sha(output / f"{label}-measured-source.json"),
            "lock_sha256": sha(source / "Cargo.lock"), "binary": str(binary),
            "binary_sha256": sha(binary), "harness": {name: overlaid[name] for name in HARNESS}}
        save(output / "receipt.json", report)
    assert report["sources"]["baseline"]["harness"] == report["sources"]["candidate"]["harness"]
    return binaries


def measure(binaries, work, output, report, smoke):
    cases = [(32, "wide"), (3, "deep")] if smoke else [(20000, "wide"), (200000, "wide"), (300, "deep")]
    for count, shape in cases:
        case = f"{count}-{shape}"
        root = work / f"fixture-{case}"
        proof = fixture(root, count, shape)
        report["fixtures"].append(proof)
        command(["stat", "-f", "-c", "%T %s %b %f", str(root)], work, output,
                f"{case}-filesystem", report)
        for round_index, order in enumerate([("baseline", "candidate"), ("candidate", "baseline")]):
            for label in order:
                run = f"{case}-round{round_index + 1}-{label}"
                data = work / f"data-{run}"
                assert not data.exists()
                result_path = output / f"{run}.json"
                assert sha(binaries[label]) == report["sources"][label]["binary_sha256"]
                env = os.environ.copy()
                env.update(DG_MEASURE_CASE=str(count), DG_MEASURE_SHAPE=shape,
                           DG_MEASURE_ROOT=str(root), DG_MEASURE_DATA=str(data),
                           DISKGRAPH_BENCHMARK_OUTPUT=str(result_path))
                result = command([str(binaries[label]), "--exact", "measure_isolated_release_fixtures",
                                  "--ignored", "--nocapture", "--test-threads=1"],
                                 work, output, run, report, env=env, timeout=1200)
                metrics = json.loads(result_path.read_text())
                assert len(metrics) == 1 and metrics[0]["files"] == count and metrics[0]["shape"] == shape
                assert metrics[0]["native_qualification"]["qualified"] is True
                report["measurements"].append({"label": label, "round": round_index + 1,
                    "fixture": case, "data_dir": str(data), "command": result["label"],
                    "metrics_sha256": sha(result_path), "metrics": metrics[0]})
                save(output / "receipt.json", report)
        verify_fixture(root, proof)
        proof["verified_unchanged_after_all_four_runs"] = True
    for fixture_name in sorted({entry["fixture"] for entry in report["measurements"]}):
        for number in (1, 2):
            pair = {entry["label"]: entry["metrics"] for entry in report["measurements"]
                    if entry["fixture"] == fixture_name and entry["round"] == number}
            report["pairs"].append({"fixture": fixture_name, "round": number,
                "candidate_over_baseline_scan_seconds": pair["candidate"]["scan_seconds"] / pair["baseline"]["scan_seconds"],
                "candidate_minus_baseline_scan_high_water_rss_bytes":
                    pair["candidate"]["process_scan_high_water_rss_bytes"] - pair["baseline"]["process_scan_high_water_rss_bytes"]})


def commit_sha(value):
    """仅接受完整不可变提交 SHA，禁止把可变 ref 或 Git revision 语法用于配对。"""
    if re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise argparse.ArgumentTypeError("requires a full lowercase 40-character commit SHA")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--smoke", action="store_true", help="32-wide/3-deep only; not release scale acceptance")
    parser.add_argument("--baseline-sha", type=commit_sha, default=BASE)
    parser.add_argument("--candidate-sha", type=commit_sha, default=NEW)
    args = parser.parse_args()
    if args.baseline_sha == args.candidate_sha:
        parser.error("baseline and candidate must be distinct immutable commits")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parents[1]
    report = {"schema_version": 1, "status": "running", "acceptance_scope": "smoke_only" if args.smoke else "release_20k_200k_300deep",
        "script_sha256": sha(Path(__file__)), "commands": [], "sources": {}, "fixtures": [],
        "measurements": [], "pairs": [], "hardware": {"uname": list(platform.uname()), "cpu_count": os.cpu_count()},
        "limitations": ["Same runner and pre-created roots; caches are not dropped or claimed cold.",
            "RSS is per-child process high-water, not Rust allocation or exclusive scan live memory.",
            "Database lengths and signed deltas are logical file sizes, not physical write bytes.",
            "Query phase includes preserved explicit rebuildable fixture evidence insertion.",
            "Qualification is persisted Linux capture; Unsupported is failure, not a performance result.",
            "Two alternating rounds describe this host only; no performance improvement threshold."]}
    save(output / "receipt.json", report)
    try:
        assert platform.system() == "Linux", "native Linux is mandatory"
        for name in ["cpuinfo", "meminfo", "mountinfo"]:
            source = Path("/proc/self/mountinfo" if name == "mountinfo" else f"/proc/{name}")
            shutil.copyfile(source, output / f"host-{name}.txt")
        command(["rustc", "-vV"], repo, output, "rustc", report)
        command(["cargo", "--version"], repo, output, "cargo-version", report)
        with tempfile.TemporaryDirectory(prefix="diskgraph-scan-cost-", dir=os.environ.get("RUNNER_TEMP")) as directory:
            work = Path(directory)
            binaries = build_pair(repo, work, output, report, args.baseline_sha, args.candidate_sha)
            measure(binaries, work, output, report, args.smoke)
        report["status"] = "passed"
    except BaseException as error:
        report["status"] = "aborted" if isinstance(error, (KeyboardInterrupt, SystemExit)) else "failed"
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        report["artifacts"] = {path.name: {"sha256": sha(path), "bytes": path.stat().st_size}
                               for path in sorted(output.iterdir()) if path.is_file() and path.name != "receipt.json"}
        save(output / "receipt.json", report)


if __name__ == "__main__":
    main()
