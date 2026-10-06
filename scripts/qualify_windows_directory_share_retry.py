#!/usr/bin/env python3
"""真实目录写句柄的共享冲突 RED/GREEN；恢复原源码，不改变共享模式。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_enumerated_absence import cargo

ROOT = Path(__file__).resolve().parents[1]
BASELINE = "45afb17731d293a61c8fb8f4cf5f8acd8fae2f50"
SOURCE = "crates/diskgraph-engine/src/live_evidence/git_directory_lease.rs"
TEST = "crates/diskgraph-engine/src/live_evidence/windows_git_share_retry_tests.rs"


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-directory-sharing"
    output.mkdir(exist_ok=False)
    path = ROOT / SOURCE
    original = path.read_bytes()
    subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = subprocess.check_output(["git", "show", f"{BASELINE}:{SOURCE}"], cwd=ROOT)
    receipt = {"candidate": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(),
               "baseline": BASELINE, "candidate_sha256": hashlib.sha256(original).hexdigest(),
               "baseline_sha256": hashlib.sha256(old).hexdigest(),
               "shared_test_sha256": hashlib.sha256((ROOT / TEST).read_bytes()).hexdigest(),
               "status": "pending"}
    try:
        path.write_bytes(old)
        code, log = cargo("actual_shared_write_handle_releases_before_original_lease_deadline", output / "red.log")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_WINDOWS_DIRECTORY_SHARING_RED_READY=1" not in log
                or "actual shared directory lease must recover after original blocker release:" not in log
                or "os error 32" not in log):
            raise RuntimeError("original lease must fail at actual released sharing-conflict assertion")
    finally:
        path.write_bytes(original)
        receipt["restored"] = path.read_bytes() == original
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["restored"]:
        raise RuntimeError("source restoration failed")
    code, log = cargo("windows_git_share_retry_tests", output / "green.log")
    if (code != 0 or "2 passed; 0 failed; 0 ignored;" not in log
            or "DG_WINDOWS_DIRECTORY_SHARING_RETRY=1" not in log):
        raise RuntimeError("actual recovery, original expiry and cancellation must execute")
    receipt["status"] = "native sharing retry verified; full regression and finite exit remain open"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
