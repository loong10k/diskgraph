#!/usr/bin/env python3
"""诊断真实Windows夹具准备的线程成本；不改变正式200k/300秒验收。"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("readonly_load", ROOT / "scripts/accept-readonly-load.py")
LOAD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LOAD)


def measure_case(workers, files):
    """创建独立普通文件，记录写入及清理成本；任何内容/路径错误直接失败。"""
    with tempfile.TemporaryDirectory(prefix="diskgraph-fixture-worker-diagnostic-") as temporary:
        root = Path(temporary)
        started = time.perf_counter()
        LOAD.create_fixture(root, files, workers=workers)
        create_seconds = time.perf_counter() - started
        # 验证不计入创建耗时；不以硬链接或少量文件替代实际写入。
        if len(list(root.iterdir())) != files:
            raise RuntimeError("fixture file count mismatch")
        for index in range(files):
            path = root / f"file-{index:06}.bin"
            if path.stat().st_nlink != 1 or path.read_bytes() != b"x" * 32:
                raise RuntimeError("fixture identity/content mismatch")
        cleanup_started = time.perf_counter()
    return {"workers": workers, "files": files, "bytes_per_file": 32,
            "create_seconds": create_seconds,
            "cleanup_seconds": time.perf_counter() - cleanup_started,
            "coverage": "exact names, count, contents and distinct links verified"}


def run(output, files=20000):
    """保存每轮实际进度与源码摘要，失败回执不能充作性能验收通过。"""
    receipt = {"status": "running", "purpose": "diagnostic only; no production SLO claim",
               "platform": platform.platform(), "python": platform.python_version(),
               "source_commit": os.environ.get("GITHUB_SHA"), "cases": [],
               "source_sha256": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                                 for path in [Path(__file__).resolve(), ROOT / "scripts/accept-readonly-load.py"]}}
    output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        output.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    save()
    try:
        # 对称顺序保留热度/次序影响，不以小夹具结果外推200k性能。
        for workers in (1, 2, 4, 4, 2, 1):
            receipt["cases"].append(measure_case(workers, files))
            save()
        receipt["status"] = "complete"
    except BaseException as error:
        receipt["status"] = "failed"
        try:
            save()
        except OSError as receipt_error:
            error.add_note(f"diagnostic receipt could not be saved: {receipt_error}")
        raise
    save()
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("native Windows required; no cross-platform qualification substitute")
    run(args.output)


if __name__ == "__main__":
    main()
