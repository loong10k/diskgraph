#!/usr/bin/env python3
"""验证外来枚举项的原完整 ID 缺失恢复；仅真正 Windows RED/GREEN 才验收。"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_legacy_api import MARKER, bridge_baseline
from qualify_windows_capacity_replay import prepare


ROOT = Path(__file__).resolve().parents[1]
BASELINE = "6c3b7295e26e31933d9258f8ee8328cc6b241f64"
SOURCES = [
    "crates/diskgraph-engine/src/live_evidence/git_private_capacity.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_private_root.rs",
    "crates/diskgraph-engine/src/live_evidence/mod.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_deletion_witness.rs",
]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def cargo(filter_name, destination):
    with destination.open("wb") as output:
        completed = subprocess.run(
            ["cargo", "test", "-p", "diskgraph-engine", "--lib", "--locked",
             filter_name, "--", "--nocapture", "--test-threads=1"],
            cwd=ROOT, stdout=output, stderr=subprocess.STDOUT, timeout=240,
            check=False,
        )
    return completed.returncode, destination.read_text(encoding="utf-8", errors="replace")


def baseline_modules(source):
    """保留当前调用图；旧实现未引用的两个新模块仅在冻结回放中移除。"""
    source = source.replace(b"\r\n", b"\n")
    for module in ("windows_git_root_parent", "windows_git_foreign_removal_witness"):
        declaration = f"#[cfg(windows)]\nmod {module};\n".encode()
        if source.count(declaration) != 1:
            raise RuntimeError("frozen absence module declaration is not unique")
        source = source.replace(declaration, b"", 1)
    return source


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-enumerated-absence"
    output.mkdir(exist_ok=False)
    original = {name: (ROOT / name).read_bytes() for name in SOURCES}
    candidate = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip()
    present = subprocess.run(["git", "cat-file", "-e", BASELINE], cwd=ROOT, check=False)
    if present.returncode:
        subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = {name: subprocess.check_output(["git", "show", f"{BASELINE}:{name}"], cwd=ROOT)
           for name in SOURCES}
    baseline_original = dict(old)
    support_current, support_old = prepare(ROOT, BASELINE, subprocess.check_output)
    original.update(support_current)
    old.update(support_old)
    module_source = "crates/diskgraph-engine/src/live_evidence/mod.rs"
    cleanup_source = "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs"
    if any(name.encode() in old[cleanup_source] for name in
           ("windows_git_root_parent", "windows_git_foreign_removal_witness")):
        raise RuntimeError("frozen cleanup unexpectedly uses new absence/root support")
    old[module_source] = baseline_modules(old[module_source])
    old[cleanup_source] = bridge_baseline(original[cleanup_source], old[cleanup_source], "directory")
    receipt = {
        "candidate": candidate, "baseline": BASELINE,
        "candidate_sources": {name: digest(data) for name, data in original.items()},
        "baseline_original_sources": {name: digest(data) for name, data in baseline_original.items()},
        "baseline_sources": {name: digest(data) for name, data in old.items()},
        # 两阶段共用同一真实游标测试；产品恢复正控只在当前实现上执行。
        "shared_support_sources": {
            name: digest((ROOT / name).read_bytes()) for name in [
                "crates/diskgraph-engine/src/live_evidence/windows_git_foreign_removal_witness.rs",
                "crates/diskgraph-engine/src/live_evidence/windows_git_root_parent.rs",
                "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor_tests.rs",
                "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup_tests.rs",
            ]
        },
        "status": "pending",
    }
    try:
        # 临时替换冻结源码；同一测试的原父/枚举 ID 和原预算不变。
        for name, data in old.items():
            (ROOT / name).write_bytes(data)
        code, log = cargo("vanished_unregistered_entry_advances_only_after_original_full_id_absence",
                          output / "red.log")
        if MARKER in log:
            raise RuntimeError("original absence primitive must not execute the new API binding")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_WINDOWS_ENUMERATED_ABSENCE_RED_READY=1" not in log
                or "full-ID absence must retire only vanished foreign entry:" not in log):
            raise RuntimeError("baseline must fail at the actual retained vanished-entry assertion")
        receipt["red"] = "original enumeration preserved moved/replacement identities; vanished entry remained stuck"
    finally:
        for name, data in original.items():
            (ROOT / name).write_bytes(data)
        receipt["restored"] = all((ROOT / name).read_bytes() == data
                                  for name, data in original.items())
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["restored"]:
        raise RuntimeError("candidate source restoration failed")
    code, log = cargo("vanished_unregistered_entry_advances_only_after_original_full_id_absence", output / "green-cursor.log")
    if (code != 0 or "1 passed; 0 failed; 0 ignored;" not in log
            or "DG_WINDOWS_ENUMERATED_ABSENCE_GREEN=1" not in log):
        raise RuntimeError("actual current cursor absence recovery must pass")
    code, log = cargo("foreign_removal_preserves_hard_links_and_outstanding_external_handle", output / "green-last-close.log")
    if (code != 0 or "1 passed; 0 failed; 0 ignored;" not in log
            or "DG_WINDOWS_FOREIGN_LINKS_AND_LAST_CLOSE=1" not in log):
        raise RuntimeError("foreign hard links and external last-close must retain original recovery")
    code, log = cargo("windows_git_cleanup_tests", output / "green-product.log")
    if (code != 0 or "5 passed; 0 failed; 0 ignored;" not in log
            or "DG_WINDOWS_FOREIGN_ENTRY_PRODUCT_RECOVERY=1" not in log):
        raise RuntimeError("all five actual productive cleanup controls must pass")
    receipt["status"] = "native red and green verified; product shutdown gate remains open"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(receipt["status"])


if __name__ == "__main__":
    main()
