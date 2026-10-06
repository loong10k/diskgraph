#!/usr/bin/env python3
"""原恢复池展开 RED/GREEN；同一真实目录、故障点与 panic 断言，字节精确还原源码。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_enumerated_absence import cargo
from qualify_windows_legacy_api import MARKER, bridge_baseline

ROOT = Path(__file__).resolve().parents[1]
BASELINE = "9b02d6c4114ac9be1a9576ec4d8fc6cdc7ec1c5f"
SOURCE = "crates/diskgraph-engine/src/probe_resource_pool.rs"
TEST = "pool_cleanup_unwind_retains_original_directory_until_actual_retry"
ANCHOR = b"            if let Some((inner, mut directory)) = work {\n"
CHECKPOINT = (b"                #[cfg(all(test, windows))]\n"
              b"                crate::probe_pool_cleanup_fault::ProbePoolCleanupFault::checkpoint();\n")


def instrument_baseline(source, candidate=b""):
    """只加入与候选共用的测试故障点，不修复旧生产路径；未知/重复锚点拒绝运行。"""
    normalized = source.replace(b"\r\n", b"\n")
    if normalized.count(ANCHOR) != 1 or b"ProbePoolCleanupFault" in normalized:
        raise RuntimeError("baseline cleanup checkpoint anchor is not unique or already instrumented")
    return bridge_baseline(candidate, normalized.replace(ANCHOR, ANCHOR + CHECKPOINT, 1), "pool")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-pool-unwind"
    output.mkdir(exist_ok=False)
    path = ROOT / SOURCE
    original = path.read_bytes()
    subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = subprocess.check_output(["git", "show", f"{BASELINE}:{SOURCE}"], cwd=ROOT)
    instrumented = instrument_baseline(old, original)
    receipt = {"candidate": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(),
               "baseline": BASELINE, "candidate_sha256": digest(original),
               "baseline_original_sha256": digest(old), "baseline_with_shared_fault_sha256": digest(instrumented),
               "shared_support_sources": {n: digest((ROOT / n).read_bytes()) for n in [
                   "crates/diskgraph-engine/src/lib.rs",
                   "crates/diskgraph-engine/src/probe_pool_cleanup_fault.rs",
                   "crates/diskgraph-engine/src/live_evidence/probe_resource_pool_tests.rs"]},
               "baseline_instrumentation": "only the same test-only panic checkpoint at the lock-free cleanup boundary",
               "status": "pending"}
    try:
        path.write_bytes(instrumented)
        code, log = cargo(TEST, output / "red.log")
        if MARKER in log:
            raise RuntimeError("pool unwind primitive must not execute the new API binding")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_PROBE_POOL_UNWIND_RED_READY=1" not in log
                or "original probe pool lost directory or stayed draining after unwind" not in log):
            raise RuntimeError("baseline must fail after real retained directory, unchanged panic, occupied/refused capacity controls")
        receipt["red"] = "same original panic and private payload preserved, but original pool cannot retry its responsibility"
    finally:
        path.write_bytes(original)
        receipt["restored"] = path.read_bytes() == original
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["restored"]:
        raise RuntimeError("source restoration failed")
    code, log = cargo(TEST, output / "green.log")
    if (code != 0 or "1 passed; 0 failed; 0 ignored;" not in log
            or "DG_PROBE_POOL_UNWIND_ORIGINAL_DIRECTORY_RESTORED=1" not in log):
        raise RuntimeError("same pool must retain and actually clean original directory after unchanged panic")
    receipt["status"] = "native original pool unwind red and green verified; general finite frontend shutdown remains separate"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(receipt["status"])


if __name__ == "__main__":
    main()
