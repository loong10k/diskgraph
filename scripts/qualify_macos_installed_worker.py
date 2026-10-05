"""冻结候选的真实root发行/普通UID运行资格；不把未运行夹具当PASS。"""
import argparse
import hashlib
import json
import os
import platform
import subprocess
import tarfile
from pathlib import Path

CANDIDATE = Path("crates/diskgraph-engine/integration_candidates/macos_installed")
ROOT_CASE = "macos_installation_root_fixture_tests::root_fresh_install_update_interruption_and_floor_bound_recovery"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def permitted(name):
    path = Path(name)
    return (not path.is_absolute() and ".." not in path.parts and name == path.as_posix()
            and (name in {"Cargo.lock", "crates/diskgraph-engine/Cargo.toml", "crates/diskgraph-engine/build.rs", "crates/diskgraph-scan-worker/Cargo.toml"}
                 or (name.startswith("crates/diskgraph-engine/src/") and path.suffix in {".rs", ".c", ".h"})
                 or (name.startswith("crates/diskgraph-engine/tests/fixtures/") and path.suffix == ".rs")
                 or (name.startswith("crates/diskgraph-scan-worker/src/") and path.suffix == ".rs")))


def mount(checkout):
    checkout = checkout.resolve(strict=True)
    archive = checkout / CANDIDATE / "candidate.tar.gz"
    manifest_path = checkout / CANDIDATE / "manifest.json"
    if manifest_path.stat().st_size > 1024 * 1024 or archive.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("candidate exceeds archive metadata budget")
    manifest = json.loads(manifest_path.read_text())
    if manifest["schema_version"] != 1 or digest(archive) != manifest["archive_sha256"]:
        raise ValueError("candidate archive identity mismatch")
    names = manifest["sources"]
    if not names or any(not permitted(name) for name in names):
        raise ValueError("candidate path escapes fixed source scope")
    with tarfile.open(archive, "r:gz") as source:
        members = source.getmembers()
        if sum(member.size for member in members) > 16 * 1024 * 1024:
            raise ValueError("candidate exceeds decoded source budget")
        if len(members) != len(names) or len({m.name for m in members}) != len(members):
            raise ValueError("duplicate or missing candidate entries")
        # 校验完整归档再修改checkout，拒绝链接、额外名称、过大文件及目录逃逸。
        contents = {}
        for member in members:
            if not member.isfile() or member.name not in names or member.size > 2 * 1024 * 1024:
                raise ValueError("invalid candidate archive member")
            data = source.extractfile(member).read()
            if hashlib.sha256(data).hexdigest() != names[member.name]:
                raise ValueError("candidate source digest mismatch")
            destination = checkout / member.name
            if destination.is_symlink():
                raise ValueError("checkout contains source symlink")
            for parent in destination.parents:
                if parent == checkout:
                    break
                if parent.is_symlink():
                    raise ValueError("checkout contains source symlink")
            contents[member.name] = data
        for name, data in contents.items():
            destination = checkout / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
    return manifest


def invoke(command, checkout, output, name, environment, timeout=900):
    with (output / (name + ".stdout")).open("wb") as stdout, (output / (name + ".stderr")).open("wb") as stderr:
        result = subprocess.run(command, cwd=checkout, env=environment, stdout=stdout, stderr=stderr, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{name} exited {result.returncode}; inspect preserved original logs")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    checkout = Path(__file__).resolve().parent.parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if platform.system() != "Darwin" or os.getuid() == 0 or os.geteuid() == 0:
        raise RuntimeError("qualification parent must be ordinary macOS UID")
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("root fixture is restricted to explicit ephemeral hosted CI")
    receipt = {"schema_version": 1, "production_acceptance": False, "status": "started", "uid": os.getuid(), "architecture": platform.machine()}
    try:
        receipt["checkout_sha"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
        receipt["rustc"] = subprocess.check_output(["rustc", "--version"], text=True).strip()
        receipt["os_version"] = platform.mac_ver()[0]
        manifest = mount(checkout)
        required_cases = {"macos_installed_worker_fixture_tests::" + name for name in (
            "macos_installed_helper_returns_complete_tree_after_real_normal_wait",
            "macos_installed_helper_original_cancel_after_birth_is_reaped",
            "macos_installed_helper_original_panic_after_birth_keeps_recovery_responsibility")}
        if set(manifest["ordinary_cases"]) != required_cases or len(manifest["ordinary_cases"]) != 3:
            raise ValueError("candidate ordinary acceptance inventory differs")
        receipt["candidate"] = manifest
        env = os.environ.copy()
        invoke(["cargo", "build", "--locked", "-p", "diskgraph-scan-worker"], checkout, output, "build-helper", env)
        helper = checkout / "target/debug/diskgraph-scan-worker"
        receipt["helper_sha256"] = digest(helper)
        receipt["helper_bytes"] = helper.stat().st_size
        invoke(["cargo", "test", "--locked", "-p", "diskgraph-engine", "--lib", "--no-run", "--message-format=json"], checkout, output, "build-fixtures", env)
        binaries = []
        for line in (output / "build-fixtures.stdout").read_text().splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "diskgraph_engine" and record.get("executable"):
                    binaries.append(Path(record["executable"]))
        if len(binaries) != 1:
            raise RuntimeError("expected one actual Engine test executable")
        binary = binaries[0]
        receipt["fixture_sha256"] = digest(binary)
        env.update(DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE=os.environ["GITHUB_RUN_ID"],
                   DISKGRAPH_MACOS_FIXTURE_HELPER=str(helper),
                   DISKGRAPH_MACOS_FIXTURE_SHA256=receipt["helper_sha256"],
                   DISKGRAPH_MACOS_FIXTURE_BYTES=str(receipt["helper_bytes"]))
        preserved = "GITHUB_ACTIONS,GITHUB_RUN_ID,RUNNER_OS,RUNNER_ENVIRONMENT,DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE,DISKGRAPH_MACOS_FIXTURE_HELPER,DISKGRAPH_MACOS_FIXTURE_SHA256,DISKGRAPH_MACOS_FIXTURE_BYTES"
        invoke(["sudo", "-n", "--preserve-env=" + preserved, str(binary), ROOT_CASE, "--exact", "--ignored", "--nocapture", "--test-threads=1"], checkout, output, "root-issuer", env)
        if "test result: ok. 1 passed; 0 failed;" not in (output / "root-issuer.stdout").read_text():
            raise RuntimeError("root fixture did not execute its exact required case")
        receipt["root_cases_passed"] = 1
        for case in manifest["ordinary_cases"]:
            invoke([str(binary), case, "--exact", "--ignored", "--nocapture", "--test-threads=1"], checkout, output, case.rsplit("::", 1)[-1], env)
            if "test result: ok. 1 passed; 0 failed;" not in (output / (case.rsplit("::", 1)[-1] + ".stdout")).read_text():
                raise RuntimeError("ordinary fixture did not execute exact required case")
        receipt["ordinary_cases_passed"] = len(manifest["ordinary_cases"])
        for name, expected in manifest["sources"].items():
            if digest(checkout / name) != expected:
                raise RuntimeError("qualification modified original candidate source")
        if digest(helper) != receipt["helper_sha256"] or digest(binary) != receipt["fixture_sha256"]:
            raise RuntimeError("qualification binary identity changed")
        receipt["status"] = "passed"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
