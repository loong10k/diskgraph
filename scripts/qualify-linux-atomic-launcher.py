#!/usr/bin/env python3
"""从精确提交隔离执行 C8 原子 launcher；不启用 Engine 或证明镜像执行策略。"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import subprocess
import sys
import tomllib
import traceback


MANIFEST = "docs/benchmarks/linux_atomic_launcher_2026_10_05_candidate.json"
NATIVE = "crates/diskgraph-engine/src/native_child/"


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(64 << 10):
            result.update(block)
    return result.hexdigest()


def shared_support():
    # 复用已审的 archive/临时目录错误保真和回收逻辑，不创建新后台 owner。
    path = Path(__file__).with_name("qualify-linux-scan-image.py")
    spec = importlib.util.spec_from_file_location("scan_image_qualification", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module, path


def qualify_namespace(environment, output):
    """拒绝不受外层监管的直接执行；这些观测不替代外层真实 wait 回执。"""
    if environment.get("DG_NATIVE_NAMESPACE_SUPERVISED") != "1":
        raise ValueError("actual outer PID namespace supervisor is required")
    if os.getpid() != 1 or os.readlink("/proc/self") != "1":
        raise ValueError("qualification must execute as the actual namespace init")
    actual = os.readlink("/proc/self/ns/pid")
    if actual != environment.get("DG_NATIVE_PID_NAMESPACE"):
        raise ValueError("actual PID namespace does not match supervisor material")
    outer = environment.get("DG_NATIVE_NAMESPACE_OUTPUT")
    if not outer or output != Path(outer).resolve() / "qualification":
        raise ValueError("output does not identify the supervised qualification directory")
    return {"actual_pid": 1, "actual_proc_self": "1", "actual_pid_namespace": actual,
            "outer_output": outer, "cleanup": "pending outer init wait receipt"}


def validate_tooling(checkout, manifest, active_qualifier):
    """从固定 archive 验证编排同路径 SHA，并拒绝运行了另一版 qualifier。"""
    tooling = manifest["qualification_tooling"]
    supervisor = manifest["required_outer_supervisor"]
    if manifest["qualifier_sha256"] != tooling["qualifier"]["sha256"]:
        raise ValueError("qualifier binding disagrees with the manifest")
    if manifest["shared_support_sha256"] != tooling["shared_support"]["sha256"]:
        raise ValueError("shared support binding disagrees with the active support contract")
    bindings = [
        (tooling["qualifier"], "scripts/qualify-linux-atomic-launcher.py"),
        (tooling["unit_tests"], "scripts/tests/test_qualify_linux_atomic_launcher.py"),
        (tooling["shared_support"], "scripts/qualify-linux-scan-image.py"),
        (supervisor["source"], "scripts/run-native-pid-namespace.py"),
        (supervisor["unit_tests"], "scripts/tests/test_native_pid_namespace.py"),
        (supervisor["workflow"], ".github/workflows/native_scan_qualification.yml"),
        (supervisor["restore_helper"], "scripts/native_pid_namespace_restore.py"),
        (supervisor["restore_tests"], "scripts/tests/test_native_pid_namespace_restore.py"),
        (supervisor["restore_error_tests"], "scripts/tests/test_native_pid_namespace_restore_errors.py"),
    ]
    for binding, expected_path in bindings:
        if binding["path"] != expected_path or not re.fullmatch(r"[0-9a-f]{64}", binding["sha256"]):
            raise ValueError("tooling binding does not identify its exact fixed source")
        source = checkout / expected_path
        if source.is_symlink() or not source.is_file() or source.stat().st_size > (1 << 20):
            raise ValueError("archived tooling is not a bounded regular source")
        if digest(source) != binding["sha256"]:
            raise ValueError(f"archived tooling digest mismatch: {expected_path}")
    active_restore = Path(__file__).with_name("native_pid_namespace_restore.py")
    if (active_restore.is_symlink() or not active_restore.is_file()
            or active_restore.stat().st_size > (1 << 20)):
        raise ValueError("active restore helper is not a bounded regular source")
    if digest(active_restore) != supervisor["restore_helper"]["sha256"]:
        raise ValueError("active restore helper digest mismatch")
    if digest(active_qualifier) != tooling["qualifier"]["sha256"]:
        raise ValueError("running qualifier differs from the exact archive and manifest")


def validate_sources(checkout, manifest):
    """完整验证以后才能改隔离副本；路径和 digest 错误不制造部分准入。"""
    if manifest["schema_version"] != 1 or manifest["expected_parent_cases"] != 17:
        raise ValueError("invalid atomic qualification manifest version or case count")
    if len(set(manifest["test_names"])) != 17:
        raise ValueError("required native cases are not unique")
    sources = manifest["sources"]
    paths, targets, modules = set(), set(), set()
    for source in sources:
        path = PurePosixPath(source["path"])
        if path.is_absolute() or ".." in path.parts or path.as_posix() in paths:
            raise ValueError("invalid or repeated source path")
        if not path.as_posix().startswith("crates/diskgraph-engine/integration_candidates/native_child/"):
            raise ValueError("source outside the frozen candidate subtree")
        paths.add(path.as_posix())
        actual = checkout / path
        if actual.is_symlink() or not actual.is_file() or actual.stat().st_size > (1 << 20):
            raise ValueError(f"not a bounded regular candidate source: {path}")
        if digest(actual) != source["sha256"]:
            raise ValueError(f"candidate source digest mismatch: {path}")
        target = source.get("target")
        if target:
            allowed = (target.startswith(NATIVE) or target == "crates/diskgraph-engine/build.rs"
                       or target == "crates/diskgraph-engine/tests/fixtures/linux_atomic_launcher_fixture.c")
            if not allowed or ".." in PurePosixPath(target).parts or target in targets:
                raise ValueError("invalid or repeated isolated target")
            targets.add(target)
        module = source.get("module")
        if module:
            if not re.fullmatch(r"[a-z][a-z0-9_]*", module) or module in modules:
                raise ValueError("invalid or repeated isolated module")
            if target != NATIVE + module + ".rs":
                raise ValueError("module does not identify its exact isolated source")
            modules.add(module)
    baseline = manifest["baseline_unix_child"]
    if digest(checkout / baseline["target"]) != baseline["sha256"]:
        raise ValueError("committed UnixChild changed; requalify the mechanical projection")
    return sources


def add_cc_dependency(cargo, lock):
    """仅引用已锁定的 cc=1，不添加包、升级版本或借用未提交 Cargo 文件。"""
    parsed = tomllib.loads(cargo)
    value = parsed.get("build-dependencies", {}).get("cc")
    if value is not None and value != "1":
        raise ValueError("baseline cc dependency differs from the frozen build contract")
    if value is None:
        header = "[build-dependencies]"
        if header in cargo:
            if cargo.count(header) != 1:
                raise ValueError("repeated build dependency section")
            position = cargo.index(header) + len(header)
            cargo = cargo[:position] + '\ncc = "1"' + cargo[position:]
        else:
            cargo += '\n[build-dependencies]\ncc = "1"\n'
    packages = tomllib.loads(lock)["package"]
    if len([item for item in packages if item["name"] == "cc"]) != 1:
        raise ValueError("exact existing cc lock package is required; no dependency resolution fallback")
    engines = [item for item in packages if item["name"] == "diskgraph-engine"]
    if len(engines) != 1:
        raise ValueError("unique Engine lock package required")
    dependencies = engines[0]["dependencies"]
    if "cc" not in dependencies:
        blocks = lock.split("[[package]]")
        indexes = [i for i, block in enumerate(blocks) if re.search(r'^name = "diskgraph-engine"$', block, re.M)]
        if len(indexes) != 1:
            raise ValueError("unique Engine lock block required")
        index = indexes[0]
        replacement = "dependencies = [\n" + "".join(
            f" {json.dumps(item)},\n" for item in sorted(dependencies + ["cc"])
        ) + "]"
        blocks[index], count = re.subn(r"dependencies = \[\n.*?\n\]", replacement,
                                      blocks[index], count=1, flags=re.S)
        if count != 1:
            raise ValueError("Engine lock dependencies are not a bounded array")
        lock = "[[package]]".join(blocks)
    return cargo, lock


def mount_candidate(checkout, manifest):
    sources = validate_sources(checkout, manifest)
    declarations = []
    module_file = checkout / (NATIVE + "mod.rs")
    original_mod = module_file.read_text()
    for source in sources:
        if source.get("module") and re.search(r"\bmod\s+" + source["module"] + r"\s*;", original_mod):
            raise ValueError(f"baseline already declares candidate module: {source['module']}")
    for source in sources:
        if source.get("target"):
            target = checkout / source["target"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(checkout / source["path"], target)
        if source.get("module"):
            declarations.append(f'#[cfg(all(test, target_os = "linux"))]\nmod {source["module"]};\n')
    declarations.append('#[cfg(all(test, target_os = "linux"))]\npub(crate) use control_write_status::ControlWriteStatus;\n')
    module_file.write_text(original_mod + "\n" + "".join(declarations))
    cargo_path = checkout / "crates/diskgraph-engine/Cargo.toml"
    lock_path = checkout / "Cargo.lock"
    before = {str(path.relative_to(checkout)): digest(path) for path in [cargo_path, lock_path]}
    cargo, lock = add_cc_dependency(cargo_path.read_text(), lock_path.read_text())
    cargo_path.write_text(cargo)
    lock_path.write_text(lock)
    return {"declarations": declarations, "dependency_before": before,
            "dependency_after": {str(path.relative_to(checkout)): digest(path) for path in [cargo_path, lock_path]}}


def finish_log(receipt, step, log_path, log, primary, support):
    """分别关闭并记录日志；有原错保留原对象，无原错传播首个材料错误。"""
    failures = []
    operations = []
    if log is not None:
        operations.append(("close", log.close))
    operations.extend([
        ("digest", lambda: step.update(log_sha256=digest(log_path))),
        ("stat", lambda: step.update(log_bytes=log_path.stat().st_size)),
    ])
    for phase, operation in operations:
        try:
            operation()
        except BaseException as error:
            failures.append(error)
            support.secondary(receipt, f'{step["name"]}_log_{phase}_error', error)
    if failures:
        step["status"] = "failed"
        if primary is None:
            raise failures[0]


def run_step(receipt, output, name, command, cwd, environment, support, timeout=1200):
    log_path = output / f"{name}.log"
    step = {"name": name, "command": command, "log": log_path.name, "status": "started"}
    receipt.setdefault("steps", []).append(step)
    log = None
    try:
        log = log_path.open("wb")
        process = subprocess.Popen(command, cwd=cwd, env=environment, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        except BaseException:
            # 本地只处置该 command leader/group；setsid 扫描者由外层 PIDns owner 回收。
            step["local_cleanup_scope"] = "command session only; outer PID namespace cleanup required"
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except BaseException as error:
                support.secondary(receipt, f"{name}_signal_error", error)
            try:
                process.wait(timeout=30)
            except BaseException as error:
                support.secondary(receipt, f"{name}_wait_error", error)
            step["exit_code"] = process.returncode
            raise
        step["exit_code"] = code
        if code != 0:
            raise subprocess.CalledProcessError(code, command)
    except BaseException:
        step["status"] = "failed"
        raise
    finally:
        original = sys.exception()
        finish_log(receipt, step, log_path, log, original, support)
    step["status"] = "completed"
    return log_path


def extract_archive(repo, checkout, output, receipt, environment, support):
    """归档使用固定提交；tar、archive、日志 close/digest 错误均按原发生顺序保真。"""
    path = output / "archive.stderr"
    step = {"name": "archive", "status": "started", "log": path.name}
    receipt.setdefault("steps", []).append(step)
    errors = archive = None
    try:
        errors = path.open("wb")
        archive = subprocess.Popen(["git", "archive", receipt["commit"]], cwd=repo,
                                   stdout=subprocess.PIPE, stderr=errors, env=environment)
        subprocess.run(["tar", "-x", "-C", str(checkout)], stdin=archive.stdout,
                       check=True, timeout=120)
    except BaseException:
        step["status"] = "failed"
        raise
    finally:
        original = sys.exception()
        failure = None
        if archive is not None:
            try:
                support.finish_archive(archive, receipt)
            except BaseException as error:
                failure = error
                support.secondary(receipt, "archive_cleanup_error", error)
            step["exit_code"] = archive.returncode
            if failure is None and archive.returncode is not None and archive.returncode != 0:
                failure = subprocess.CalledProcessError(archive.returncode, ["git", "archive", receipt["commit"]])
        try:
            finish_log(receipt, step, path, errors, original or failure, support)
        except BaseException as error:
            failure = failure or error
        if original is None and failure is not None:
            step["status"] = "failed"
            raise failure
    if archive.returncode != 0:
        step["status"] = "failed"
        raise subprocess.CalledProcessError(archive.returncode, ["git", "archive", receipt["commit"]])
    step["status"] = "completed"


def object_evidence(target, output, receipt, environment, support):
    archives = list(target.glob("debug/build/diskgraph-engine-*/out/libdiskgraph_linux_atomic_spawn.a"))
    if len(archives) != 1:
        raise ValueError("expected exactly one newly built native spawn archive")
    objects = list(archives[0].parent.glob("*linux_atomic_spawn.o"))
    if len(objects) != 1:
        raise ValueError("expected exactly one newly built native spawn object")
    directory = output / "objects"
    directory.mkdir()
    for source in [archives[0], objects[0]]:
        shutil.copyfile(source, directory / source.name)
    native = directory / objects[0].name
    undefined = run_step(receipt, output, "native-nm-undefined", ["nm", "-u", str(native)], output, environment, support, 30)
    disassembly = run_step(receipt, output, "native-objdump", ["objdump", "-dr", str(native)], output, environment, support, 30)
    members = run_step(receipt, output, "native-archive-members", ["ar", "t", str(directory / archives[0].name)], output, environment, support, 30)
    raw = disassembly.read_text()
    if undefined.read_text().strip():
        raise ValueError("native child object has unresolved symbols; inspect original nm output")
    if "dg_linux_atomic_spawn" not in raw:
        raise ValueError("actual raw child birth function absent from disassembly")
    if re.search(r"TLS|tpidr|%[fg]s:|__stack_chk|__asan|__tsan|__ubsan|mcount|__gcov", raw, re.I):
        raise ValueError("native child object contains TLS/instrumentation evidence")
    if members.read_text().splitlines() != [objects[0].name]:
        raise ValueError("spawn archive membership does not match the inspected object")
    receipt["native_objects"] = [{"path": str(path.relative_to(output)), "sha256": digest(path),
                                  "bytes": path.stat().st_size} for path in directory.iterdir()]
    receipt["object_check"] = "no undefined symbols/TLS/instrumentation detected; original disassembly retained for independent review"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "linux" or os.uname().machine not in ("x86_64", "aarch64"):
        parser.error("requires actual native Linux x86_64/aarch64; unsupported is not a passed qualification")
    if os.environ.get("CARGO_ENCODED_RUSTFLAGS") or os.environ.get("CARGO_BUILD_TARGET"):
        parser.error("encoded flags or a cross target override would invalidate native qualification")
    repo = Path(__file__).resolve().parents[1]
    output = args.output_dir.resolve()
    namespace = qualify_namespace(os.environ, output)
    output.mkdir(parents=True, exist_ok=False)
    support, support_path = shared_support()
    receipt = {"schema_version": 1, "scope": __doc__, "status": "incomplete",
               "platform": sys.platform, "architecture": os.uname().machine,
               "script_sha256": digest(Path(__file__)), "shared_support_sha256": digest(support_path),
               "namespace": namespace}
    environment = os.environ.copy()
    environment["RUSTFLAGS"] = environment.get("RUSTFLAGS", "") + " -Dwarnings"
    receipt["rustflags"] = environment["RUSTFLAGS"]
    try:
        commit_log = run_step(receipt, output, "captured-commit", ["git", "rev-parse", "HEAD"], repo, environment, support, 30)
        receipt["commit"] = commit_log.read_text().strip()
        if not re.fullmatch(r"[0-9a-f]{40}", receipt["commit"]):
            raise ValueError("git did not return one exact commit")
        with support.isolated_checkout(output, receipt) as checkout:
            extract_archive(repo, checkout, output, receipt, environment, support)
            manifest_path = checkout / MANIFEST
            if manifest_path.stat().st_size > (128 << 10):
                raise ValueError("candidate manifest exceeds the fixed input budget")
            manifest = json.loads(manifest_path.read_text())
            validate_tooling(checkout, manifest, Path(__file__))
            if manifest["shared_support_sha256"] != receipt["shared_support_sha256"]:
                raise ValueError("qualification support differs from the committed source binding")
            receipt.update(candidate_manifest_sha256=digest(manifest_path), sources=manifest["sources"])
            receipt["isolated_mount"] = mount_candidate(checkout, manifest)
            target = checkout / "target-atomic-qualification"
            environment["CARGO_TARGET_DIR"] = str(target)
            fixtures = output / "fixtures"
            fixtures.mkdir()
            source = checkout / "crates/diskgraph-engine/tests/fixtures/linux_atomic_launcher_fixture.c"
            receipt["fixture_source_sha256"] = digest(source)
            receipt["spawn_source_sha256"] = digest(checkout / (NATIVE + "linux_atomic_spawn.c"))
            for number in (1, 2):
                destination = fixtures / f"image-{number}"
                run_step(receipt, output, f"fixture-image-{number}", ["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", "-pthread", f"-DIMAGE_ID={number}", str(source), "-o", str(destination)], checkout, environment, support, 120)
            receipt["fixture_images"] = [{"path": str(path.relative_to(output)), "sha256": digest(path),
                                         "bytes": path.stat().st_size} for path in fixtures.iterdir()]
            environment["DG_LINUX_ATOMIC_LAUNCH_FIXTURE"] = str(fixtures / "image-1")
            environment["DG_LINUX_ATOMIC_LAUNCH_REPLACEMENT"] = str(fixtures / "image-2")
            command = ["cargo", "test", "--offline", "--locked", "-p", "diskgraph-engine", "--lib", "native_child::linux_atomic_launcher_", "--"]
            inventory = run_step(receipt, output, "native-test-inventory", command + ["--list"], checkout, environment, support)
            names = [line.split(": test")[0].rsplit("::", 1)[-1] for line in inventory.read_text().splitlines() if line.endswith(": test")]
            if sorted(names) != sorted(manifest["test_names"]):
                raise ValueError("actual compiled inventory does not match the 17 required parent cases")
            receipt["compiled_inventory"] = names
            object_evidence(target, output, receipt, environment, support)
            log = run_step(receipt, output, "native-tests", command + ["--nocapture", "--test-threads=1"], checkout, environment, support)
            if not re.search(rb"test result: ok\. 17 passed; 0 failed; 0 ignored;", log.read_bytes()):
                raise ValueError("actual serial execution did not pass all 17 required parent cases")
            receipt.update(status="component_tests_passed_awaiting_outer_cleanup", executed_parent_cases=17)
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
