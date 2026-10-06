#!/usr/bin/env python3
"""真实旧兼容清理忽略期限的 RED，与同 owner 绝对期限保留/恢复 GREEN。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_enumerated_absence import cargo
from qualify_windows_legacy_api import MARKER, SEAL_MARKER, bridge_baseline, prepare_pool_deadline_support

ROOT = Path(__file__).resolve().parents[1]
BASELINE = "46fededd709ee614eab5083c746399dde05664cf"
SOURCE = "crates/diskgraph-engine/src/probe_resource_pool.rs"
TEST = "expired_probe_recovery_keeps_actual_directory_and_capacity_until_live_retry"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-probe-deadline"
    output.mkdir(exist_ok=False)
    path = ROOT / SOURCE
    original = path.read_bytes()
    support_original, support_adapted = prepare_pool_deadline_support(ROOT)
    subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = subprocess.check_output(["git", "show", f"{BASELINE}:{SOURCE}"], cwd=ROOT)
    instrumented = bridge_baseline(original, old, "pool")
    if instrumented == old:
        raise RuntimeError("deadline RED requires the explicit actual legacy drain delegate")
    receipt = {"candidate": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(),
               "baseline": BASELINE, "candidate_sha256": digest(original),
               "baseline_original_sha256": digest(old), "baseline_legacy_delegate_sha256": digest(instrumented),
               "shared_sources": {n: digest((ROOT / n).read_bytes()) for n in [
                   "crates/diskgraph-engine/src/probe_recovery.rs",
                   "crates/diskgraph-engine/src/live_evidence/probe_resource_pool_tests.rs",
                   "crates/diskgraph-engine/src/live_evidence/git_private_directory_owner.rs",
                   "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs",
                   "scripts/qualify_windows_legacy_api.py"]}, "status": "pending"}
    receipt["unreachable_deadline_support"] = {
        "original_sources": {name: digest(data) for name, data in support_original.items()},
        "baseline_sources": {name: digest(data) for name, data in support_adapted.items()},
        "policy": "only unreachable new methods omitted; compatibility cleanup body unchanged, strict warnings retained",
    }
    try:
        path.write_bytes(instrumented)
        for name, data in support_adapted.items():
            (ROOT / name).write_bytes(data)
        code, log = cargo(TEST, output / "red.log")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_EXPIRED_PROBE_RECOVERY_RED_READY=1" not in log or MARKER not in log or SEAL_MARKER in log
                or "DG_LEGACY_EXPIRED_ORIGINAL_ACTUALLY_REMOVED=1" not in log
                or "expired recovery must retain the original directory without deletion: Ok(true)" not in log):
            raise RuntimeError("legacy deadline RED must actually remove the same original despite expiry")
        receipt["red"] = "real original disposal completed through old compatibility drain despite expired input; original behavior delegate observed"
    finally:
        path.write_bytes(original)
        for name, data in support_original.items():
            (ROOT / name).write_bytes(data)
        receipt["restored"] = (path.read_bytes() == original
                               and all((ROOT / name).read_bytes() == data for name, data in support_original.items()))
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["restored"]:
        raise RuntimeError("source restoration failed")
    code, log = cargo(TEST, output / "green.log")
    if (code != 0 or "1 passed; 0 failed; 0 ignored;" not in log or MARKER in log or SEAL_MARKER in log
            or "DG_EXPIRED_PROBE_RECOVERY_RETAINS_ORIGINAL_THEN_ACTUALLY_REMOVES=1" not in log):
        raise RuntimeError("same original must survive expired calls and actually complete on explicit live recovery")
    receipt["status"] = "native expired attempt preservation and actual same-owner live retry verified; finite frontend shutdown remains open"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(receipt["status"])


if __name__ == "__main__":
    main()
