#!/usr/bin/env python3
"""以同一真实测试验证旧 owner 重试 RED 和当前实现 GREEN；不替换原生接口。"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from qualify_windows_legacy_api import MARKER, SEAL_MARKER, prepare_pool_deadline_support, strip_unused_pool_deadline_method
from qualify_windows_capacity_replay import prepare as prepare_capacity_replay
from qualify_windows_pool_unwind import instrument_baseline


ROOT = Path(__file__).resolve().parents[1]
BASELINE = "296e533f5a262833d64da145d8dca3fb0824c57b"
SOURCES = [
    "crates/diskgraph-engine/src/probe_resource_pool.rs",
    "crates/diskgraph-engine/src/probe_directory_binding.rs",
    "crates/diskgraph-engine/src/live_evidence/git_private_directory.rs",
    "crates/diskgraph-engine/src/live_evidence/git_private_allocation.rs",
    "crates/diskgraph-engine/src/live_evidence/git_private_capacity.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_cleanup.rs",
    "crates/diskgraph-engine/src/live_evidence/windows_git_directory_cursor.rs",
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


def main():
    if sys.platform != "win32":
        raise RuntimeError("actual Windows execution required")
    output = Path(os.environ["RUNNER_TEMP"]) / "diskgraph-windows-directory-retry"
    output.mkdir(exist_ok=False)
    original = {name: (ROOT / name).read_bytes() for name in SOURCES}
    candidate = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip()
    present = subprocess.run(["git", "cat-file", "-e", BASELINE], cwd=ROOT, check=False)
    if present.returncode:
        subprocess.run(["git", "fetch", "--depth=1", "origin", BASELINE], cwd=ROOT, check=True)
    old = {name: subprocess.check_output(["git", "show", f"{BASELINE}:{name}"], cwd=ROOT)
           for name in SOURCES}
    baseline_original = dict(old)
    support_original, support_adapted = prepare_pool_deadline_support(ROOT)
    original.update(support_original)
    creation_original, creation_adapted = prepare_capacity_replay(ROOT, BASELINE, subprocess.check_output)
    original.update(creation_original)
    old.update(creation_adapted)
    modules = "crates/diskgraph-engine/src/live_evidence/mod.rs"
    # 冻结清理/枚举不引用这两个后续新增类型；不改变当前产品模块或警告策略。
    for declaration in (
        b"#[cfg(windows)]\nmod windows_git_foreign_removal_witness;\n",
        b"#[cfg(windows)]\nmod windows_git_root_parent;\n",
    ):
        if old[modules].count(declaration) != 1:
            raise RuntimeError("frozen directory unreachable module boundary is not unique")
        old[modules] = old[modules].replace(declaration, b"", 1)
    # 两种不可达支持剔除组合在同一 owner 副本上，不相互覆盖恢复调用边界。
    for name in support_adapted:
        if name in SOURCES:
            support_adapted[name] = strip_unused_pool_deadline_method(old[name])
        elif name in creation_adapted:
            support_adapted[name] = strip_unused_pool_deadline_method(creation_adapted[name])
    pool_source = "crates/diskgraph-engine/src/probe_resource_pool.rs"
    old[pool_source] = instrument_baseline(old[pool_source], original[pool_source])
    old.update(support_adapted)
    receipt = {
        "candidate": candidate, "baseline": BASELINE,
        "baseline_test_binding": "same test-only unarmed pool checkpoint; directory retry targets do not arm it; original owner retry unchanged",
        "candidate_sources": {name: digest(data) for name, data in original.items()},
        "baseline_original_sources": {name: digest(data) for name, data in baseline_original.items()},
        "baseline_sources": {name: digest(data) for name, data in old.items()},
        "unreachable_deadline_support": {
            "original_sources": {name: digest(data) for name, data in support_original.items()},
            "baseline_sources": {name: digest(data) for name, data in support_adapted.items()},
            "policy": "cleanup/cursor/capacity/allocation frozen to baseline; current owner omits unsupported creation recovery and unreachable deadline methods",
        },
        # 两阶段共用同一真实测试与未启用的故障点；所有临时实现差异由上方源码摘要记录。
        "shared_support_sources": {
            name: digest((ROOT / name).read_bytes()) for name in [
                "crates/diskgraph-engine/src/live_evidence/probe_resource_pool_tests.rs",
                "crates/diskgraph-engine/src/probe_pool_cleanup_fault.rs",
            ]
        },
        "status": "pending",
    }
    try:
        # 目录与分配接口冻结为同一提交；同一真实测试及原预算不变，不伪造缺失方法。
        for name, data in old.items():
            (ROOT / name).write_bytes(data)
        code, log = cargo("explicit_directory_retry_reborrows_only_original_live_session_owner",
                          output / "red.log")
        if MARKER in log or SEAL_MARKER in log:
            raise RuntimeError("original directory retry must not execute the new API binding")
        if (code == 0 or "0 passed; 1 failed; 0 ignored;" not in log
                or "DG_WINDOWS_ORIGINAL_DIRECTORY_RETRY_RED_READY=1" not in log
                or "DG_WINDOWS_ORIGINAL_DIRECTORY_RETRY_RED_CLEANUP=1" not in log
                or 'Err` value: "private Git owner retained for recovery"' not in log):
            raise RuntimeError("old source must fail at the actual original-owner retry assertion")
        receipt["red"] = "actual denied deletion and retained original slot; exact retry failed"
    finally:
        for name, data in original.items():
            (ROOT / name).write_bytes(data)
        receipt["restored"] = all((ROOT / name).read_bytes() == data
                                  for name, data in original.items())
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["restored"]:
        raise RuntimeError("candidate source restoration failed")
    code, log = cargo("directory_retry", output / "green.log")
    if (code != 0 or MARKER in log or SEAL_MARKER in log or "2 passed; 0 failed; 0 ignored;" not in log
            or "DG_WINDOWS_ORIGINAL_DIRECTORY_EXPLICIT_RETRY=1" not in log
            or "DG_WINDOWS_RELEASED_SESSION_CANNOT_REBORROW_DIRECTORY=1" not in log):
        raise RuntimeError("both actual current owner retry cases must execute successfully")
    receipt["status"] = "native red and green verified; product shutdown gate remains open"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(receipt["status"])


if __name__ == "__main__":
    main()
