"""真实旧实现失败比较；编译错误、其他 panic 或零用例不算目标 RED。"""
import argparse
import json
import os
import platform
import re
import subprocess
from pathlib import Path
import qualify_macos_installed_worker as shared
import qualify_windows_scan_cleanup as windows

CANDIDATE = Path("crates/diskgraph-engine/integration_candidates/windows_prepared_birth_baseline")
CASE = "native_child::windows::windows_birth_recovery_tests::actual_create_process_failure_retains_external_job_until_observed_cleanup"


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
    output = Path(args.output_dir)
    output.mkdir(parents=True, exist_ok=True)
    receipt = {"production_acceptance": False, "status": "started", "case": CASE}
    try:
        windows.check_platform(platform.system(), os.environ.get("GITHUB_ACTIONS"), os.environ.get("RUNNER_ENVIRONMENT"))
        receipt["checkout_sha"] = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        receipt["candidate"] = manifest = shared.mount(checkout, CANDIDATE)
        if manifest.get("baseline_expected_failure_case") != CASE:
            raise ValueError("baseline target case differs")
        env = os.environ.copy()
        shared.invoke(["cargo", "test", "--locked", "-p", "diskgraph-engine", "--lib", "--no-run", "--message-format=json"],
                      checkout, output, "build-baseline", env)
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
            shared.invoke([str(binaries[0]), CASE, "--exact", "--nocapture"], checkout, output, "actual-lost-owner", env, timeout=120)
        except RuntimeError:
            check_target_red((output / "actual-lost-owner.stdout").read_text(), (output / "actual-lost-owner.stderr").read_text())
        else:
            raise RuntimeError("old source unexpectedly passed lost-owner regression")
        receipt["status"] = "verified_target_red"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
