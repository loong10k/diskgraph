#!/usr/bin/env python3
"""隔离提交中真实执行 memfd 六案；保留原 flags，外层回收证据仍独立。"""
import argparse
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import traceback


MANIFEST = "docs/benchmarks/linux_memfd_execution_2026_10_05_candidate.json"
ATOMIC_MANIFEST = "docs/benchmarks/linux_atomic_launcher_2026_10_05_candidate.json"
NATIVE = "crates/diskgraph-engine/src/native_child/"
ATOMIC_PATH = Path(__file__).with_name("qualify-linux-atomic-launcher.py")
SPEC = importlib.util.spec_from_file_location("atomic_qualification_support", ATOMIC_PATH)
ATOMIC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ATOMIC)


def bounded_manifest(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > (128 << 10):
        raise ValueError("manifest is not a bounded regular source")
    return json.loads(path.read_text())


def validate_profile(checkout, manifest, active):
    """核新 profile 与根已冻结的 current atomic 来源，不重绑定或修改旧材料。"""
    fixed = [
        (manifest["qualifier"], "scripts/qualify-linux-memfd-execution.py"),
        (manifest["unit_tests"], "scripts/tests/test_qualify_linux_memfd_execution.py"),
    ]
    for binding, expected in fixed:
        if binding["path"] != expected or not re.fullmatch(r"[0-9a-f]{64}", binding["sha256"]):
            raise ValueError("profile tooling does not identify its exact source")
        path = checkout / expected
        if path.is_symlink() or not path.is_file() or path.stat().st_size > (1 << 20):
            raise ValueError("profile tooling is not a bounded regular source")
        if ATOMIC.digest(path) != binding["sha256"]:
            raise ValueError("archived memfd tooling digest mismatch")
    if ATOMIC.digest(active) != manifest["qualifier"]["sha256"]:
        raise ValueError("running memfd qualifier differs from archive and manifest")
    helper = manifest["profile_helper"]
    expected = "scripts/native_memfd_profile.py"
    if helper["path"] != expected or not re.fullmatch(r"[0-9a-f]{64}", helper["sha256"]):
        raise ValueError("profile helper does not identify its exact source")
    for source in (checkout / expected, Path(__file__).with_name("native_memfd_profile.py")):
        if source.is_symlink() or not source.is_file() or source.stat().st_size > (1 << 20):
            raise ValueError("profile helper is not a bounded regular source")
        if ATOMIC.digest(source) != helper["sha256"]:
            raise ValueError("profile helper active or archived digest mismatch")
    atomic_path = checkout / ATOMIC_MANIFEST
    original = bounded_manifest(atomic_path)
    if ATOMIC.digest(atomic_path) != manifest["atomic_manifest_sha256"]:
        raise ValueError("archived atomic manifest differs from the fixed baseline")
    ATOMIC.validate_tooling(checkout, original, ATOMIC_PATH)
    return original


def validate_additional_sources(checkout, manifest):
    """先验全部 sealed4+memfd6，之后才挂载；不修改镜像 flags 或 frozen tests。"""
    if manifest["schema_version"] != 1 or manifest["expected_parent_cases"] != 6:
        raise ValueError("invalid memfd profile version or count")
    sources = manifest["sources"]
    if len(sources) != 10:
        raise ValueError("exact sealed4 and memfd6 sources required")
    names, paths = set(), set()
    for source in sources:
        path = PurePosixPath(source["path"])
        if path.is_absolute() or ".." in path.parts or str(path) in paths:
            raise ValueError("invalid or repeated candidate path")
        if not str(path).startswith("crates/diskgraph-engine/integration_candidates/native_child/"):
            raise ValueError("candidate outside fixed subtree")
        paths.add(str(path))
        actual = checkout / path
        if actual.is_symlink() or not actual.is_file() or actual.stat().st_size > (1 << 20):
            raise ValueError("candidate is not a bounded regular source")
        if ATOMIC.digest(actual) != source["sha256"]:
            raise ValueError(f"candidate digest mismatch: {path}")
        module = source.get("module")
        if module:
            if not re.fullmatch(r"[a-z][a-z0-9_]*", module) or module in names or path.name != module + ".rs":
                raise ValueError("invalid or repeated exact candidate module")
            names.add(module)
    if manifest["policy_fixture"] not in paths or not manifest["policy_fixture"].endswith("/linux_memfd_policy_namespace.c"):
        raise ValueError("policy fixture does not identify its exact admitted C source")
    groups = manifest["test_groups"]
    if [len(group["names"]) for group in groups] != [2, 1, 3]:
        raise ValueError("exact existing filters must preserve actual2/1/3 cases")
    all_names = [name for group in groups for name in group["names"]]
    if len(set(all_names)) != 6:
        raise ValueError("six unique exact cases required")
    for group in groups:
        if group["filter"] not in names or any(not name.startswith("native_child::" + group["filter"] + "::") for name in group["names"]):
            raise ValueError("case does not belong to its existing exact module")
    return sources


def mount_profile(checkout, manifest, atomic_manifest):
    sources = validate_additional_sources(checkout, manifest)
    module_path = checkout / (NATIVE + "mod.rs")
    original = module_path.read_text()
    for source in sources:
        if source.get("module") and re.search(r"\bmod\s+" + source["module"] + r"\s*;", original):
            raise ValueError("baseline already declares additional candidate module")
    result = ATOMIC.mount_candidate(checkout, atomic_manifest)
    declarations = []
    for source in sources:
        if source.get("module"):
            shutil.copyfile(checkout / source["path"], checkout / (NATIVE + source["module"] + ".rs"))
            declarations.append(f'#[cfg(all(test, target_os = "linux"))]\nmod {source["module"]};\n')
    module_path.write_text(module_path.read_text() + "\n" + "".join(declarations))
    result["memfd_declarations"] = declarations
    return result


def run_cases(checkout, output, receipt, environment, support, groups):
    """不重试；Rust 目标失败后仍执行其余原 filters，并最终传播首个实际失败。"""
    failures = []
    results = []
    for group in groups:
        command = ["cargo", "test", "--offline", "--locked", "-p", "diskgraph-engine", "--lib",
                   group["filter"], "--", "--nocapture", "--test-threads=1"]
        name = "native-" + group["filter"]
        try:
            log = ATOMIC.run_step(receipt, output, name, command, checkout, environment, support)
            code = 0
        except subprocess.CalledProcessError as error:
            failures.append(error)
            code = error.returncode
            log = output / (name + ".log")
        except BaseException as error:
            if failures:
                support.secondary(receipt, name + "_later_operation_error", error)
                receipt["native_results"] = results
                raise failures[0]
            raise
        try:
            raw = log.read_bytes()
        except BaseException as error:
            results.append({"filter": group["filter"], "exit_code": code,
                            "qualification": "native log unavailable"})
            receipt["native_results"] = results
            if failures:
                support.secondary(receipt, name + "_log_read_error", error)
                raise failures[0]
            raise
        summaries = re.findall(rb"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", raw)
        if not summaries:
            failure = ValueError("actual parent test summary missing: " + group["filter"])
            if failures:
                support.secondary(receipt, name + "_summary_error", failure)
            failures.append(failure)
            results.append({"filter": group["filter"], "exit_code": code, "qualification": "missing parent execution summary"})
            continue
        passed, failed, ignored = map(int, summaries[-1])
        result = {"filter": group["filter"], "exit_code": code, "passed": passed, "failed": failed,
                  "ignored": ignored, "namespace_missing_qualification_text": b"missing_qualification" in raw}
        results.append(result)
        if passed + failed != len(group["names"]) or ignored != 0 or (code == 0 and failed != 0):
            failures.append(ValueError("actual case count/exit inconsistent: " + group["filter"]))
    receipt["native_results"] = results
    if failures:
        raise failures[0]
    if sum(item["passed"] for item in results) != 6:
        raise ValueError("actual execution did not pass exactly six cases")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "linux" or os.uname().machine not in ("x86_64", "aarch64"):
        parser.error("requires actual native Linux x86_64/aarch64; no skip or emulation")
    if os.environ.get("CARGO_ENCODED_RUSTFLAGS") or os.environ.get("CARGO_BUILD_TARGET"):
        parser.error("encoded flags/cross target invalidate native qualification")
    output = args.output_dir.resolve()
    namespace = ATOMIC.qualify_namespace(os.environ, output)
    output.mkdir(parents=True, exist_ok=False)
    support, support_path = ATOMIC.shared_support()
    repo = Path(__file__).resolve().parents[1]
    receipt = {"schema_version": 1, "scope": __doc__, "status": "incomplete", "namespace": namespace,
               "platform": sys.platform, "architecture": os.uname().machine,
               "script_sha256": ATOMIC.digest(Path(__file__)), "shared_support_sha256": ATOMIC.digest(support_path)}
    environment = os.environ.copy()
    environment["RUSTFLAGS"] = environment.get("RUSTFLAGS", "") + " -Dwarnings"
    receipt["rustflags"] = environment["RUSTFLAGS"]
    try:
        commit_log = ATOMIC.run_step(receipt, output, "captured-commit", ["git", "rev-parse", "HEAD"], repo, environment, support, 30)
        receipt["commit"] = commit_log.read_text().strip()
        if not re.fullmatch(r"[0-9a-f]{40}", receipt["commit"]):
            raise ValueError("exact captured commit required")
        with support.isolated_checkout(output, receipt) as checkout:
            ATOMIC.extract_archive(repo, checkout, output, receipt, environment, support)
            manifest = bounded_manifest(checkout / MANIFEST)
            atomic_manifest = validate_profile(checkout, manifest, Path(__file__))
            if receipt["shared_support_sha256"] != atomic_manifest["shared_support_sha256"]:
                raise ValueError("running shared support differs from exact archive")
            receipt.update(candidate_manifest_sha256=ATOMIC.digest(checkout / MANIFEST), sources=manifest["sources"],
                           atomic_manifest_sha256=manifest["atomic_manifest_sha256"], atomic_sources=atomic_manifest["sources"])
            receipt["isolated_mount"] = mount_profile(checkout, manifest, atomic_manifest)
            target = checkout / "target-memfd-qualification"
            environment["CARGO_TARGET_DIR"] = str(target)
            fixtures = output / "fixtures"
            fixtures.mkdir()
            source = checkout / "crates/diskgraph-engine/tests/fixtures/linux_atomic_launcher_fixture.c"
            receipt["spawn_source_sha256"] = ATOMIC.digest(checkout / (NATIVE + "linux_atomic_spawn.c"))
            builds = [(f"image-{number}", ["-pthread", f"-DIMAGE_ID={number}"], source) for number in (1, 2)]
            policy = checkout / manifest["policy_fixture"]
            builds.append(("memfd-policy-namespace", [], policy))
            for name, flags, file in builds:
                command = ["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror"] + flags + [str(file), "-o", str(fixtures / name)]
                ATOMIC.run_step(receipt, output, "fixture-" + name, command, checkout, environment, support, 120)
            receipt["fixture_sources"] = [{"path": str(path.relative_to(checkout)), "sha256": ATOMIC.digest(path)} for path in [source, policy]]
            receipt["fixture_images"] = [{"path": str(path.relative_to(output)), "sha256": ATOMIC.digest(path), "bytes": path.stat().st_size} for path in fixtures.iterdir()]
            environment.update(DG_LINUX_ATOMIC_LAUNCH_FIXTURE=str(fixtures / "image-1"),
                               DG_LINUX_ATOMIC_LAUNCH_REPLACEMENT=str(fixtures / "image-2"),
                               DG_MEMFD_POLICY_NAMESPACE_FIXTURE=str(fixtures / "memfd-policy-namespace"))
            for group in manifest["test_groups"]:
                command = ["cargo", "test", "--offline", "--locked", "-p", "diskgraph-engine", "--lib", group["filter"], "--", "--list"]
                log = ATOMIC.run_step(receipt, output, "inventory-" + group["filter"], command, checkout, environment, support)
                names = [line[:-6] for line in log.read_text().splitlines() if line.endswith(": test")]
                if sorted(names) != sorted(group["names"]):
                    raise ValueError("actual inventory mismatch: " + group["filter"])
            receipt["compiled_inventory"] = [name for group in manifest["test_groups"] for name in group["names"]]
            ATOMIC.object_evidence(target, output, receipt, environment, support)
            run_cases(checkout, output, receipt, environment, support, manifest["test_groups"])
            receipt.update(status="component_tests_passed_awaiting_outer_cleanup", executed_parent_cases=6)
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["primary_error"] = {"type": type(error).__name__, "repr": repr(error)}
        receipt["traceback"] = traceback.format_exc()
        raise
    finally:
        original = sys.exception()
        try:
            (output / "receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
        except BaseException as error:
            if original is None:
                raise
            support.secondary(receipt, "receipt_write_error", error)


if __name__ == "__main__":
    main()
