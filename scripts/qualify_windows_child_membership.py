#!/usr/bin/env python3
"""真实原子项跨父目录改名 RED/GREEN；不改变最终原生移除证明。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_enumerated_absence import cargo

ROOT = Path(__file__).resolve().parents[1]
BASELINE = "9b02d6c4114ac9be1a9576ec4d8fc6cdc7ec1c5f"
SOURCE = "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor.rs"
TEST = "cleanup_child_delete_lease_freezes_original_parent_until_actual_removal"


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-child-membership"
    output.mkdir(exist_ok=False)
    path = ROOT / SOURCE
    original = path.read_bytes()
    subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = subprocess.check_output(["git", "show", f"{BASELINE}:{SOURCE}"], cwd=ROOT)
    receipt = {"candidate": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(),
               "baseline": BASELINE,
               "candidate_sha256": hashlib.sha256(original).hexdigest(),
               "baseline_sha256": hashlib.sha256(old).hexdigest(),
               "shared_test_sha256": hashlib.sha256((ROOT / "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor_tests.rs").read_bytes()).hexdigest(),
               "status": "pending"}
    try:
        path.write_bytes(old)
        code, log = cargo(TEST, output / "red.log")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_ORIGINAL_CHILD_ASSOCIATION_RED_READY=1" not in log
                or "original child DELETE lease must freeze parent association: Ok(())" not in log):
            raise RuntimeError("baseline must permit actual cross-parent move of same full-ID held cleanup child")
        receipt["red"] = "original child really reparented while old DELETE lease remained held"
    finally:
        path.write_bytes(original)
        receipt["restored"] = path.read_bytes() == original
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["restored"]:
        raise RuntimeError("candidate restoration failed")
    code, log = cargo(TEST, output / "green.log")
    if (code != 0 or "1 passed; 0 failed; 0 ignored;" not in log
            or "DG_ORIGINAL_CHILD_DELETE_LEASE_AND_ACTUAL_REMOVAL=1" not in log):
        raise RuntimeError("same original child must reject reparenting and complete actual removal")
    receipt["status"] = "native original child membership red and green verified; general frontend shutdown remains separate"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(receipt["status"])


if __name__ == "__main__":
    main()
