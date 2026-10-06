#!/usr/bin/env python3
"""首次外来对象的原注册拒绝语义：保留真实游标、哨兵和原生失败结果。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_enumerated_absence import cargo

ROOT = Path(__file__).resolve().parents[1]
BASELINE = "ee39a5ef38f495349bda3f68cfeec9c8efc89381"
SOURCE = "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor.rs"
TEST = "cleanup_child_requires_original_owner_registration_and_rejects_foreign_replacement"


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-registration-rejection"
    output.mkdir(exist_ok=False)
    path = ROOT / SOURCE
    original = path.read_bytes()
    subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = subprocess.check_output(["git", "show", f"{BASELINE}:{SOURCE}"], cwd=ROOT)
    receipt = {"candidate": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(), "baseline": BASELINE,
               "source": SOURCE, "candidate_sha256": hashlib.sha256(original).hexdigest(), "baseline_sha256": hashlib.sha256(old).hexdigest(),
               "shared_test_sha256": hashlib.sha256((ROOT / "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor_tests.rs").read_bytes()).hexdigest(),
               "status": "pending"}
    try:
        path.write_bytes(old)
        code, log = cargo(TEST, output / "red.log")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_ORIGINAL_FOREIGN_REGISTRATION_RED_READY=1" not in log
                or "private Git first foreign error must preserve registration rejection:" not in log):
            raise RuntimeError("baseline must fail at original not-registered diagnostic after intact sentinel and retained cursor")
        receipt["red"] = "original foreign sentinel and cursor retained; original registration diagnostic missing"
    finally:
        path.write_bytes(original)
        receipt["restored"] = path.read_bytes() == original
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2)+"\n")
    if not receipt["restored"]:
        raise RuntimeError("source restoration failed")
    code, log = cargo(TEST, output / "green.log")
    if code != 0 or "1 passed; 0 failed; 0 ignored;" not in log or "DG_CLEANUP_REQUIRES_ORIGINAL_OWNER_LEDGER=1" not in log:
        raise RuntimeError("both original foreign refusal/registration and replacement identity controls must pass")
    receipt["status"] = "native original registration rejection red and green verified"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2)+"\n")
    print(receipt["status"])

if __name__ == "__main__":
    main()
