#!/usr/bin/env python3
"""拒绝篡改私有证据后仍实际清理原对象；原生 RED/GREEN 保留完整身份门禁。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_enumerated_absence import cargo
from qualify_windows_legacy_api import MARKER, bridge_baseline
from qualify_windows_capacity_replay import prepare

ROOT = Path(__file__).resolve().parents[1]
BASELINE = "ee39a5ef38f495349bda3f68cfeec9c8efc89381"
SOURCES = [
    "crates/diskgraph-engine/src/live_evidence/git_private_capacity.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs",
]
TEST = "modified_original_private_artifact_is_rejected_as_evidence_but_actually_disposed"


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-private-disposal"
    output.mkdir(exist_ok=False)
    original = {n: (ROOT / n).read_bytes() for n in SOURCES}
    subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = {n: subprocess.check_output(["git", "show", f"{BASELINE}:{n}"], cwd=ROOT) for n in SOURCES}
    baseline_original = dict(old)
    support_current, support_old = prepare(ROOT, BASELINE, subprocess.check_output)
    original.update(support_current)
    old.update(support_old)
    cleanup_source = "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs"
    old[cleanup_source] = bridge_baseline(original[cleanup_source], old[cleanup_source], "directory")
    receipt = {"candidate": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(), "baseline": BASELINE,
               "candidate_sources": {n: hashlib.sha256(b).hexdigest() for n,b in original.items()},
               "baseline_original_sources": {n: hashlib.sha256(b).hexdigest() for n,b in baseline_original.items()},
               "baseline_sources": {n: hashlib.sha256(b).hexdigest() for n,b in old.items()},
               "shared_test_sources": {n: hashlib.sha256((ROOT / n).read_bytes()).hexdigest() for n in [
                   "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor_tests.rs",
                   "crates/diskgraph-engine/src/live_evidence/git_callback_tests.rs"]}, "status": "pending"}
    try:
        for n,b in old.items():
            (ROOT / n).write_bytes(b)
        code, log = cargo(TEST, output / "red.log")
        if MARKER in log:
            raise RuntimeError("original primitive must not execute the new API binding")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_MODIFIED_PRIVATE_IDENTITY_REJECTION_RED_READY=1" not in log
                or "modified original private artifact must remain disposable:" not in log):
            raise RuntimeError("baseline must reject original disposal after genuine identity control and version-strict rejection")
        receipt["red"] = "same original full identity and changed version proven; productive rejection also blocked artifact disposal"
    finally:
        for n,b in original.items():
            (ROOT / n).write_bytes(b)
        receipt["restored"] = all((ROOT / n).read_bytes() == b for n,b in original.items())
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2)+"\n")
    if not receipt["restored"]:
        raise RuntimeError("source restoration failed")
    code, log = cargo(TEST, output / "green-disposal.log")
    if code != 0 or "1 passed; 0 failed; 0 ignored;" not in log or "DG_MODIFIED_PRIVATE_DISPOSAL_WITHOUT_EVIDENCE_TRUST=1" not in log:
        raise RuntimeError("actual original disposal must finish without accepting modified evidence")
    code, log = cargo("native_same_size_private_index_rewrite_is_not_a_complete_public_sample", output / "green-public-rejection.log")
    if code != 0 or "1 passed; 0 failed; 0 ignored;" not in log:
        raise RuntimeError("unchanged public private-index rewrite rejection and actual cleanup contract must pass")
    receipt["status"] = "native modified-evidence rejection and original disposal verified; other permanent cleanup failures remain separate"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2)+"\n")
    print(receipt["status"])

if __name__ == "__main__":
    main()
