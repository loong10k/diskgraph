"""真实旧实现失败比较；编译错误、其他 panic 或零用例不算目标 RED。"""
import argparse
import json
import os
import platform
import re
import subprocess
import tarfile
import tempfile
import shutil
from contextlib import contextmanager
from pathlib import Path
import qualify_macos_installed_worker as shared
import qualify_windows_scan_cleanup as windows

CANDIDATE = Path("crates/diskgraph-engine/integration_candidates/windows_prepared_birth_baseline")
CASE = "native_child::windows::windows_birth_recovery_tests::actual_create_process_failure_retains_external_job_until_observed_cleanup"

DEADLINE_CASE = "native_child::windows::windows_probe_recovery_tests::expired_managed_probe_transfers_original_owner_without_legacy_wait"


@contextmanager
def isolated_source(checkout, candidate=CANDIDATE):
    """导出完整历史 workspace 后覆盖已冻结探针，禁止污染当前产品依赖。"""
    checkout = checkout.resolve(strict=True)
    if candidate not in (CANDIDATE, windows.CANDIDATE, windows.BASELINE_CANDIDATE):
        raise ValueError("unsupported qualification source scope")
    manifest = json.loads((checkout / candidate / "manifest.json").read_text())
    base = manifest["base_ref"]
    if not isinstance(base, str) or not re.fullmatch(r"[0-9a-f]{40}", base):
        raise ValueError("baseline requires an exact historical commit")
    kind = subprocess.check_output(["git", "cat-file", "-t", base], cwd=checkout, text=True).strip()
    if kind != "commit":
        raise ValueError("baseline source identity is not a commit")
    with tempfile.TemporaryDirectory(prefix="diskgraph-original-source-") as temporary:
        directory = Path(temporary)
        source = directory / "source"
        source.mkdir()
        archive = directory / "original.tar"
        subprocess.run(["git", "archive", "--format=tar", "--output=" + str(archive), base],
                       cwd=checkout, check=True)
        with tarfile.open(archive) as original:
            original.extractall(source, filter="data")
        destination = source / candidate
        destination.mkdir(parents=True, exist_ok=True)
        for name in ("candidate.tar.gz", "manifest.json"):
            shutil.copyfile(checkout / candidate / name, destination / name)
        mounted = shared.mount(source, candidate)
        yield source, mounted


def check_deadline_red(stdout, stderr):
    if ("test " + DEADLINE_CASE + " ... " not in stdout
            or "test result: FAILED. 0 passed; 1 failed; 0 ignored;" not in stdout
            or "expired product probe must not enter legacy wait" not in stderr
            or not re.search(r"\bDG_EXPIRED_PROBE_BOUNDARY=1\b", stdout)):
        raise RuntimeError("old source did not fail at the exact expired cleanup boundary")


def check_target_red(stdout, stderr):
    if ("test " + CASE + " ... " not in stdout
            or "test result: FAILED. 0 passed; 1 failed; 0 ignored;" not in stdout
            or "actual failed CreateProcess must retain original Job" not in stderr
            or not re.search(r"\bDG_ACTUAL_CREATE_PROCESS_FAILURE=\d+\b", stdout)):
        raise RuntimeError("old source did not fail at the exact lost-owner assertion")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()
    checkout = Path.cwd()
    output = Path(args.output_dir).resolve()
    output.mkdir(parents=True, exist_ok=True)
    receipt = {"production_acceptance": False, "status": "started", "case": CASE}
    try:
        windows.check_platform(platform.system(), os.environ.get("GITHUB_ACTIONS"), os.environ.get("RUNNER_ENVIRONMENT"))
        receipt["checkout_sha"] = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        with isolated_source(checkout) as (source, manifest):
            receipt["candidate"] = manifest
            receipt["baseline_workspace_commit"] = manifest["base_ref"]
            if manifest.get("baseline_expected_failure_case") != CASE:
                raise ValueError("baseline target case differs")
            env = os.environ.copy()
            env["CARGO_TARGET_DIR"] = str(output.parent / "windows-qualification-target")
            shared.invoke(["cargo", "test", "--locked", "-p", "diskgraph-engine", "--lib", "--no-run", "--message-format=json"],
                          source, output, "build-baseline", env)
            binaries = []
            for line in (output / "build-baseline.stdout").read_text().splitlines():
                if line.startswith("{"):
                    item = json.loads(line)
                    if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "diskgraph_engine" and item.get("executable"):
                        binaries.append(Path(item["executable"]))
            if len(binaries) != 1:
                raise RuntimeError("expected one compiled original-source fixture executable")
            receipt["fixture_sha256"] = shared.digest(binaries[0])
            try:
                shared.invoke([str(binaries[0]), CASE, "--exact", "--nocapture"], source, output, "actual-lost-owner", env, timeout=120)
            except RuntimeError:
                check_target_red((output / "actual-lost-owner.stdout").read_text(), (output / "actual-lost-owner.stderr").read_text())
            else:
                raise RuntimeError("old source unexpectedly passed lost-owner regression")
            if manifest.get("baseline_deadline_failure_case") != DEADLINE_CASE:
                raise ValueError("baseline deadline target differs")
            try:
                shared.invoke([str(binaries[0]), DEADLINE_CASE, "--exact", "--nocapture"], source, output, "actual-expired-cleanup", env, timeout=120)
            except RuntimeError:
                check_deadline_red((output / "actual-expired-cleanup.stdout").read_text(), (output / "actual-expired-cleanup.stderr").read_text())
            else:
                raise RuntimeError("old source unexpectedly passed expired cleanup regression")
            receipt["deadline_case"] = DEADLINE_CASE
            receipt["status"] = "verified_target_red"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
