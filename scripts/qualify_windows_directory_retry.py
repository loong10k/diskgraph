#!/usr/bin/env python3
"""以同一真实测试验证旧 owner 重试 RED 和当前实现 GREEN；不替换原生接口。"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
BASELINE = "296e533f5a262833d64da145d8dca3fb0824c57b"
SOURCES = [
    "crates/diskgraph-engine/src/probe_resource_pool.rs",
    "crates/diskgraph-engine/src/probe_directory_binding.rs",
    "crates/diskgraph-engine/src/live_evidence/git_private_directory.rs",
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
    receipt = {
        "candidate": candidate, "baseline": BASELINE,
        "candidate_sources": {name: digest(data) for name, data in original.items()},
        "baseline_sources": {name: digest(data) for name, data in old.items()},
        # 两阶段共用当前诊断投影与同一真实测试；只有上面的 owner 重试实现被替换。
        "shared_support_sources": {
            name: digest((ROOT / name).read_bytes()) for name in [
                "crates/diskgraph-engine/src/live_evidence/git_private_directory_owner.rs",
                "crates/diskgraph-engine/src/live_evidence/probe_resource_pool_tests.rs",
            ]
        },
        "status": "pending",
    }
    try:
        # 仅临时替换这三个实现文件；同一新测试、真实 Windows OS 和原预算保持不变。
        for name, data in old.items():
            (ROOT / name).write_bytes(data)
        code, log = cargo("explicit_directory_retry_reborrows_only_original_live_session_owner",
                          output / "red.log")
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
    if (code != 0 or "2 passed; 0 failed; 0 ignored;" not in log
            or "DG_WINDOWS_ORIGINAL_DIRECTORY_EXPLICIT_RETRY=1" not in log
            or "DG_WINDOWS_RELEASED_SESSION_CANNOT_REBORROW_DIRECTORY=1" not in log):
        raise RuntimeError("both actual current owner retry cases must execute successfully")
    receipt["status"] = "native red and green verified; product shutdown gate remains open"
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(receipt["status"])


if __name__ == "__main__":
    main()
