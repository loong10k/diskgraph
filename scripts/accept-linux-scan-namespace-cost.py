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
BASELINE_DIAGNOSTIC_REVISION = "2a2f8281f9f211b6632bdb26afdcd4a4fb21a44d"
BASELINE_ROOT = "crates/diskgraph-engine/src/native_process/linux_scan_namespace.rs"
BASELINE_NATIVE = "crates/diskgraph-engine/src/native_process/linux_open.rs"
BASELINE_ROOT_SHA = "4787a0d69af24d0914e8a394716a4ea2dac3ba4b9b029fac3864e60701fbf9bc"
BASELINE_NATIVE_SHA = "b569eda8885214eb66f9fc026daa9751e504f88ac6cb7f347a5f2e0441d635d8"
# 仅原失败分支取证；成功调用、判断顺序、解析标志及错误分类保持原样。
BASELINE_DIAGNOSTIC_EDITS = [('        )?;\n        same_identity(&self.anchor, &current_anchor, check)?;',
  '        ).map_err(|failure| {\n'
  '                let errno = std::io::Error::last_os_error().raw_os_error();\n'
  '                eprintln!("DG_BASELINE_ANCHOR_OPEN_FAILURE class={failure:?} '
  'errno={errno:?}");\n'
  '                failure\n'
  '            })?;\n'
  '        same_identity(&self.anchor, &current_anchor, check)?;'),
 ('            )?;\n            same_identity(original, &current, check)?;',
  '            ).map_err(|failure| {\n'
  '                let errno = std::io::Error::last_os_error().raw_os_error();\n'
  '                eprintln!("DG_BASELINE_COMPONENT_OPEN_FAILURE class={failure:?} '
  'errno={errno:?}");\n'
  '                failure\n'
  '            })?;\n'
  '            same_identity(original, &current, check)?;'),
 ('unique_mount(original)?',
  'unique_mount(original).map_err(|failure| {\n'
  '        eprintln!("DG_BASELINE_HELD_MOUNT_FAILURE class={failure:?}");\n'
  '        failure\n'
  '    })?'),
 ('unique_mount(current)?',
  'unique_mount(current).map_err(|failure| {\n'
  '        eprintln!("DG_BASELINE_REOPENED_MOUNT_FAILURE class={failure:?}");\n'
  '        failure\n'
  '    })?'),
 ('let before = original.metadata().map_err(|_| Failure::Unavailable)?;',
  'let before = original.metadata().map_err(|failure| {\n'
  '        eprintln!("DG_BASELINE_HELD_METADATA_FAILURE errno={:?}", failure.raw_os_error());\n'
  '        Failure::Unavailable\n'
  '    })?;'),
 ('let after = current.metadata().map_err(|_| Failure::Unavailable)?;',
  'let after = current.metadata().map_err(|failure| {\n'
  '        eprintln!("DG_BASELINE_REOPENED_METADATA_FAILURE errno={:?}", failure.raw_os_error());\n'
  '        Failure::Unavailable\n'
  '    })?;'),
 ('        return Err(Failure::Conflict);',
  '        eprintln!("DG_BASELINE_ROOT_IDENTITY_FAILURE");\n'
  '        return Err(Failure::Conflict);')]


def baseline_diagnostic_source(root, native):
    """校验冻结实现后仅覆盖失败分支；返回可逆源码，不放宽基线算法。"""
    if (hashlib.sha256(root).hexdigest() != BASELINE_ROOT_SHA
            or hashlib.sha256(native).hexdigest() != BASELINE_NATIVE_SHA):
        raise RuntimeError("frozen baseline diagnostic source mismatch")
    # last_error 只读取线程 errno；绑定完整 native 文件以免后续调用污染 errno。
    for original, diagnostic in BASELINE_DIAGNOSTIC_EDITS:
        before, after = original.encode(), diagnostic.encode()
        if root.count(before) != 1:
            raise RuntimeError("frozen baseline diagnostic anchor mismatch")
        root = root.replace(before, after)
    return root

def apply_baseline_diagnostic(source, label, revision):
    """仅指定历史侧写入自建副本；候选和其他提交不读取或修改源码。"""
    if label != "baseline" or revision != BASELINE_DIAGNOSTIC_REVISION:
        return None
    root = source / BASELINE_ROOT
    original = root.read_bytes()
    native = (source / BASELINE_NATIVE).read_bytes()
    changed = baseline_diagnostic_source(original, native)
    root.write_bytes(changed)
    return {"mode": "failure-only diagnostic; original algorithm unchanged",
            "path": BASELINE_ROOT, "original_sha256": hashlib.sha256(original).hexdigest(),
            "measured_sha256": hashlib.sha256(changed).hexdigest(),
            "native_sha256": hashlib.sha256(native).hexdigest()}


HARNESS = ["crates/diskgraph-engine/tests/hardening_benchmark.rs",
           "crates/diskgraph-engine/tests/benchmark_support/mod.rs",
           "crates/diskgraph-engine/tests/benchmark_support/peak_memory.rs",
           "crates/diskgraph-engine/tests/benchmark_support/benchmark_engine.rs",
           "crates/diskgraph-engine/tests/benchmark_support/namespace_cost.rs",
           "crates/diskgraph-engine/tests/benchmark_support/scan_failure_diagnostic.rs"]
ENV_KEYS = ["PATH", "RUSTFLAGS", "RUSTUP_TOOLCHAIN", "CARGO_BUILD_JOBS",
            "CARGO_TARGET_DIR", "TMPDIR", "LANG", "LC_ALL", "DG_MEASURE_CASE",
            "DG_MEASURE_SHAPE", "DG_MEASURE_ROOT", "DG_MEASURE_DATA", "DISKGRAPH_BENCHMARK_OUTPUT",
            "DISKGRAPH_SCAN_WORKER_PATH", "DISKGRAPH_SCAN_WORKER_SHA256", "DISKGRAPH_SCAN_WORKER_BYTES"]


ADAPTER = "crates/diskgraph-engine/tests/benchmark_support/benchmark_engine.rs"
LEGACY_ADAPTER = """use diskgraph_engine::{Engine, EngineConfig, EngineError};
use std::sync::Arc;
/// 仅历史基线的原Engine入口；来源：该冻结提交的原公开API，不用于候选资格。
pub(crate) struct BenchmarkEngine { pub(crate) engine: Arc<Engine> }
impl BenchmarkEngine {
    /// 参数：原基线配置；返回：原行为引擎，不增加worker或替换历史扫描路径。
    pub(crate) fn open(config: EngineConfig) -> Result<Self, EngineError> {
        Ok(Self { engine: Arc::new(Engine::open(config)?) })
    }
}
"""


def worker_environment(source):
    """每侧只消费自己的实际Cargo产物，替换/尺寸变更均拒绝测量。"""
    worker = source.get("worker")
    if worker is None:
        return {}
    image = Path(worker["binary"])
    if (not image.is_absolute() or image.is_symlink() or not image.is_file()
            or image.stat().st_size != worker["bytes"] or sha(image) != worker["binary_sha256"]):
        raise RuntimeError("measured worker differs from original build artifact")
    return {"DISKGRAPH_SCAN_WORKER_PATH": str(image),
            "DISKGRAPH_SCAN_WORKER_SHA256": worker["binary_sha256"],
            "DISKGRAPH_SCAN_WORKER_BYTES": str(worker["bytes"])}


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
    if process.returncode is not None:
        raise RuntimeError("cannot signal a reaped process group leader")
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()



def observe_exit(process, timeout):
    """只观察不回收leader；保留PID身份直到组信号完成，沿用原命令期限。"""
    deadline = time.monotonic() + timeout
    while True:
        observed = os.waitid(os.P_PID, process.pid,
                             os.WEXITED | os.WNOHANG | os.WNOWAIT)
        if observed is not None and observed.si_pid == process.pid:
            return observed.si_status if observed.si_code == os.CLD_EXITED else -observed.si_status
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired(process.args, timeout)
        time.sleep(min(0.01, remaining))


def command(argv, cwd, output, label, report, env=None, timeout=1200, required=True):
    if not all(hasattr(os, name) for name in ("waitid", "WNOWAIT", "WEXITED", "WNOHANG", "P_PID")):
        raise RuntimeError("non-reaping child exit observation is unsupported")
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
            observed_exit = observe_exit(process, timeout)
            if observed_exit != 0:
                # 主进程失败不代表同组helper已经退出；停止同组后代，保留原退出码。
                terminate_group(process)
                record["failure_terminated_process_group"] = True
            record["exit_code"] = process.wait()
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


def build_pair(repo, work, output, report, baseline=BASE, candidate=None):
    if candidate is None:
        candidate = commit_sha(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip())
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
        diagnostic = apply_baseline_diagnostic(source, label, revision)
        for name in HARNESS:
            destination = source / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(repo / name, destination)
        native_host = "pub use scan_worker_settings::ScanWorkerSettings;" in (source / "crates/diskgraph-engine/src/lib.rs").read_text()
        if not native_host:
            if label != "baseline":
                raise RuntimeError("candidate must implement explicit native scan host")
            (source / ADAPTER).write_text(LEGACY_ADAPTER)
        overlaid = inventory(source)
        save(output / f"{label}-measured-source.json", overlaid)
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(work / f"target-{label}")
        worker = None
        if native_host:
            worker_build = command(["cargo", "build", "--release", "--locked", "-p", "diskgraph-scan-worker", "--message-format=json"],
                                   source, output, f"{label}-worker-build", report, env=env, timeout=1800)
            images = []
            for line in (output / worker_build["stdout"]).read_text().splitlines():
                if not line.startswith("{"):
                    continue
                record = json.loads(line)
                if (record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "diskgraph-scan-worker"
                        and record.get("target", {}).get("kind") == ["bin"] and record.get("executable")):
                    images.append(Path(record["executable"]))
            if len(images) != 1:
                raise RuntimeError("one actual release scan worker artifact required")
            image = images[0]
            worker = {"binary": str(image), "binary_sha256": sha(image), "bytes": image.stat().st_size,
                      "commit": revision, "source_manifest_sha256": sha(output / f"{label}-original-source.json")}
            worker_environment({"worker": worker})
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
            "binary_sha256": sha(binary), "harness": {name: overlaid[name] for name in HARNESS if name != ADAPTER},
            "adapter_sha256": overlaid[ADAPTER], "adapter_mode": "explicit_native_host" if native_host else "historical_engine_open",
            "worker": worker, "baseline_failure_diagnostic": diagnostic}
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
                for key in ("DISKGRAPH_SCAN_WORKER_PATH", "DISKGRAPH_SCAN_WORKER_SHA256", "DISKGRAPH_SCAN_WORKER_BYTES"):
                    env.pop(key, None)
                env.update(worker_environment(report["sources"][label]))
                env.update(DG_MEASURE_CASE=str(count), DG_MEASURE_SHAPE=shape,
                           DG_MEASURE_ROOT=str(root), DG_MEASURE_DATA=str(data),
                           DISKGRAPH_BENCHMARK_OUTPUT=str(result_path))
                result = command([str(binaries[label]), "--exact", "measure_isolated_release_fixtures",
                                  "--ignored", "--nocapture", "--test-threads=1"],
                                 work, output, run, report, env=env, timeout=1200)
                worker_environment(report["sources"][label])
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
                    memory_difference(pair["candidate"]["process_scan_high_water_rss_bytes"], pair["baseline"]["process_scan_high_water_rss_bytes"])})


def memory_difference(candidate, baseline):
    """参数为独立高水位字节观测；返回有效整数差值，未知或无效观测为 None。"""
    if type(candidate) is not int or type(baseline) is not int or min(candidate, baseline) < 0:
        return None
    return candidate - baseline


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
    parser.add_argument("--candidate-sha", type=commit_sha, help="defaults to HEAD resolved once to an immutable full SHA")
    args = parser.parse_args()
    if args.candidate_sha is None:
        args.candidate_sha = commit_sha(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=Path(__file__).resolve().parents[1], text=True).strip())
    if args.baseline_sha == args.candidate_sha:
        parser.error("baseline and candidate must be distinct immutable commits")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    repo = Path(__file__).resolve().parents[1]
    report = {"schema_version": 1, "status": "running", "acceptance_scope": "smoke_only" if args.smoke else "release_20k_200k_300deep",
        "script_sha256": sha(Path(__file__)), "commands": [], "sources": {}, "fixtures": [],
        "measurements": [], "pairs": [], "hardware": {"uname": list(platform.uname()), "cpu_count": os.cpu_count()},
        "limitations": ["Same runner and pre-created roots; caches are not dropped or claimed cold.",
            "Host and reaped child RSS are separate high waters; their sum is conservative and not simultaneous live RSS.",
            "Historical baseline uses its original Engine entry; candidate uses explicit process host, with distinct adapter fingerprints.",
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
