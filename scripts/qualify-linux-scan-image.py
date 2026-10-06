#!/usr/bin/env python3
"""在隔离的当前提交副本执行 Linux 内核密封测试，不声明 Engine 已接入。"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile


def secondary(receipt, key, error):
    """记录次要失败；记录自身失败不能替换正在传播的原错误。"""
    try:
        receipt[key] = repr(error)
        print(f"secondary {key}: {error!r}", file=sys.stderr)
    except BaseException:
        pass


@contextmanager
def isolated_checkout(output, receipt):
    checkout = Path(tempfile.mkdtemp(prefix="checkout-", dir=output))
    try:
        yield checkout
    finally:
        original = sys.exception()
        try:
            shutil.rmtree(checkout)
        except BaseException as error:
            if original is None:
                raise
            secondary(receipt, "checkout_cleanup_error", error)


def finish_archive(process, receipt):
    """回收原 archive 进程；清理错误不覆盖正在传播的 tar 原错。"""
    primary = sys.exception()
    failures = []

    def cleanup(operation, key):
        try:
            operation()
        except BaseException as error:
            failures.append(error)
            secondary(receipt, key, error)

    cleanup(process.stdout.close, "archive_pipe_close_error")
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired as error:
        failures.append(error)
        secondary(receipt, "archive_wait_timeout", error)
        cleanup(process.kill, "archive_kill_error")
        cleanup(lambda: process.wait(timeout=30), "archive_reap_error")
    except BaseException as error:
        failures.append(error)
        secondary(receipt, "archive_wait_error", error)
        cleanup(process.kill, "archive_kill_error")
        cleanup(lambda: process.wait(timeout=30), "archive_reap_error")
    if primary is None and failures:
        raise failures[0]


def current_sources(checkout):
    """只记录当前提交已集成的源码；缺失或重复声明拒绝执行，不挂载候选。"""
    relative = Path("crates/diskgraph-engine/src/native_child")
    native = checkout / relative
    modules = native / "mod.rs"
    declarations = modules.read_text()
    names = ("linux_scan_image", "linux_scan_image_tests",
             "linux_scan_image_fixture", "linux_scan_image_error")
    sources = [modules]
    for name in names:
        if len(re.findall(rf"^mod {name};$", declarations, re.MULTILINE)) != 1:
            raise RuntimeError(f"current module must be declared exactly once: {name}")
        sources.append(native / f"{name}.rs")
    return [{"path": str(path.relative_to(checkout)),
             "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for path in sources]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("requires an actual native Linux host; no emulation or skip")
    repo = Path(__file__).resolve().parents[1]
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {
        "scope": "native sealed-image component, not production Engine integration",
        "commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=repo, text=True
        ).strip(),
        "platform": sys.platform,
        "architecture": os.uname().machine,
        "status": "incomplete",
    }
    try:
        with isolated_checkout(output, receipt) as checkout:
            with (output / "archive.stderr").open("wb") as errors:
                archive = subprocess.Popen(
                    ["git", "archive", receipt["commit"]], cwd=repo,
                    stdout=subprocess.PIPE, stderr=errors,
                )
                try:
                    subprocess.run(
                        ["tar", "-x", "-C", str(checkout)],
                        stdin=archive.stdout, check=True, timeout=120,
                    )
                finally:
                    finish_archive(archive, receipt)
                if archive.returncode != 0:
                    raise RuntimeError(f"git archive failed: {archive.returncode}")
            receipt["sources"] = current_sources(checkout)
            receipt["source_mode"] = "current committed source, no candidate overlay"
            environment = os.environ.copy()
            environment["CARGO_TARGET_DIR"] = str(repo / "target")
            command = [
                "cargo", "test", "--locked", "-p", "diskgraph-engine", "--lib",
                "native_child::linux_scan_image_tests", "--", "--nocapture",
            ]
            receipt["command"] = command
            with (output / "native-tests.log").open("wb") as log:
                process = subprocess.Popen(command, cwd=checkout, env=environment,
                                           stdout=log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
                try:
                    returncode = process.wait(timeout=1200)
                except BaseException:
                    # 原 leader 尚未 wait 消费；终止独立构建组并回收，不遗留 rustc/test 后代。
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    except BaseException as error:
                        secondary(receipt, "cleanup_signal_error", error)
                    try:
                        process.wait(timeout=30)
                    except BaseException as error:
                        secondary(receipt, "cleanup_wait_error", error)
                    raise
            receipt["exit_code"] = returncode
            raw = (output / "native-tests.log").read_bytes()
            receipt["log_sha256"] = hashlib.sha256(raw).hexdigest()
            if returncode != 0:
                raise RuntimeError(f"native sealed-image qualification failed: {returncode}")
            if b"test result: ok. 11 passed; 0 failed; 0 ignored" not in raw:
                raise RuntimeError("did not execute all 11 required native tests")
            receipt["status"] = "component_passed"
    finally:
        original = sys.exception()
        try:
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        except BaseException as error:
            if original is None:
                raise
            secondary(receipt, "receipt_write_error", error)


if __name__ == "__main__":
    main()
