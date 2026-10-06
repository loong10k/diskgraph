#!/usr/bin/env python3
"""真实跨父目录原根的 RED/GREEN；不以路径缺失代替最终删除证明。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_enumerated_absence import cargo
from qualify_windows_legacy_api import MARKER, bridge_baseline

ROOT = Path(__file__).resolve().parents[1]
BASELINE = "37c663cba7269b251678a5797fe99e2b3317c71a"
SOURCES = [
    "crates/diskgraph-engine/src/live_evidence/mod.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_private_root.rs",
]


def baseline_modules(source):
    """仅旧回放不编译其不存在的 binder；当前测试导出及其余模块完全保留。"""
    source = source.replace(b"\r\n", b"\n")
    declaration = b"#[cfg(windows)]\nmod windows_git_root_parent;\n"
    if source.count(declaration) != 1:
        raise RuntimeError("original root binder module declaration is not unique")
    return source.replace(declaration, b"", 1)


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-root-parent"
    output.mkdir(exist_ok=False)
    original = {name: (ROOT / name).read_bytes() for name in SOURCES}
    subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = {name: subprocess.check_output(["git", "show", f"{BASELINE}:{name}"], cwd=ROOT) for name in SOURCES}
    baseline_original = dict(old)
    cleanup_source = "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs"
    if b"windows_git_root_parent" in old[cleanup_source]:
        raise RuntimeError("frozen cleanup unexpectedly depends on the new root binder")
    module_source = "crates/diskgraph-engine/src/live_evidence/mod.rs"
    old[module_source] = baseline_modules(original[module_source])
    old[cleanup_source] = bridge_baseline(original[cleanup_source], old[cleanup_source], "directory")
    receipt = {"candidate": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(), "baseline": BASELINE,
               "candidate_sources": {n: hashlib.sha256(b).hexdigest() for n,b in original.items()},
               "baseline_original_sources": {n: hashlib.sha256(b).hexdigest() for n,b in baseline_original.items()},
               "baseline_sources": {n: hashlib.sha256(b).hexdigest() for n,b in old.items()},
               "shared_test_sha256": hashlib.sha256((ROOT / "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup_tests.rs").read_bytes()).hexdigest(),
               "shared_support_sources": {n: hashlib.sha256((ROOT / n).read_bytes()).hexdigest() for n in [
                   "crates/diskgraph-engine/src/live_evidence/windows_git_root_parent.rs",
                   "crates/diskgraph-engine/src/live_evidence/windows_git_private_root_tests.rs"]},
               "baseline_module_origin": "current module graph minus unused new binder only; current test exports preserved",
               "status": "pending"}
    try:
        for name, data in old.items():
            (ROOT / name).write_bytes(data)
        code, log = cargo("original_root_cross_parent_cleanup_preserves_foreign_replacement", output / "red.log")
        if MARKER in log:
            raise RuntimeError("original primitive must not execute the new API binding")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_CROSS_PARENT_ORIGINAL_ROOT_RED_READY=1" not in log
                or "actual cross-parent original root removal must complete:" not in log):
            raise RuntimeError("original creation-parent subscription must fail at actual cross-parent completion")
        receipt["red"] = "original root actually moved; original creation-parent subscription could not confirm final removal"
    finally:
        for name, data in original.items():
            (ROOT / name).write_bytes(data)
        receipt["restored"] = all((ROOT / n).read_bytes() == b for n,b in original.items())
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2)+"\n")
    if not receipt["restored"]:
        raise RuntimeError("source restoration failed")
    code, log = cargo("original_root_cross_parent_cleanup_preserves_foreign_replacement", output / "green.log")
    if code != 0 or "1 passed; 0 failed; 0 ignored;" not in log or "DG_CROSS_PARENT_ORIGINAL_ROOT_GREEN=1" not in log:
        raise RuntimeError("actual original root removal and retained foreign replacement must pass")
    code, log = cargo("delete_lease_freezes_original_parent_association", output / "green-membership-lease.log")
    if code != 0 or "1 passed; 0 failed; 0 ignored;" not in log or "DG_ORIGINAL_ROOT_DELETE_LEASE_FREEZES_MEMBERSHIP=1" not in log:
        raise RuntimeError("real DELETE lease must prevent reparenting until release")
    receipt["status"] = "native cross-parent red and green verified; finite shutdown remains open"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2)+"\n")
    print(receipt["status"])

if __name__ == "__main__":
    main()
