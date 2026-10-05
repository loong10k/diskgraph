"""执行自有 macOS 原生进程限额夹具，保留真实错误和原进程 wait。"""
import argparse
import errno
import hashlib
import json
import pathlib
import platform
import resource
import subprocess
import time


def qualify(binary, source, output, commit):
    output.mkdir(parents=True, exist_ok=True)
    before = resource.getrlimit(resource.RLIMIT_NPROC)
    started = time.monotonic()
    process = subprocess.Popen([str(binary)], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               env={"PATH": "/usr/bin:/bin"}, start_new_session=True)
    timed_out = False
    stdout = stderr = b""
    primary_error = None
    cleanup_errors = []
    try:
        stdout, stderr = process.communicate(timeout=max(0, 20 - (time.monotonic() - started)))
    except BaseException as error:
        timed_out = isinstance(error, subprocess.TimeoutExpired)
        primary_error = {"kind": type(error).__name__, "errno": getattr(error, "errno", None)}
        stdout = getattr(error, "output", None) or b""
        stderr = getattr(error, "stderr", None) or b""
    finally:
        # 失败救援不刷新资格期限；异常或救援失败仍保留原主错误并拒绝能力通过。
        if process.returncode is None:
            try:
                process.kill()
            except BaseException as error:
                cleanup_errors.append({"kind": type(error).__name__, "errno": getattr(error, "errno", None)})
            try:
                stdout, stderr = process.communicate(timeout=2)
            except BaseException as error:
                cleanup_errors.append({"kind": type(error).__name__, "errno": getattr(error, "errno", None)})
            try:
                process.wait(timeout=2)
            except BaseException as error:
                cleanup_errors.append({"kind": type(error).__name__, "errno": getattr(error, "errno", None)})
    artifact_errors = []
    for name, data in [("stdout", stdout), ("stderr", stderr)]:
        try:
            (output / f"{name}.bin").write_bytes(data[:2048])
        except OSError as error:
            artifact_errors.append({"stage": name, "kind": type(error).__name__, "errno": error.errno})
    digests = {}
    for name, path in [("source", source), ("binary", binary)]:
        try:
            digests[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError as error:
            digests[name] = None
            artifact_errors.append({"stage": name, "kind": type(error).__name__, "errno": error.errno})
    elapsed = time.monotonic() - started
    record = {
        "commit": commit,
        "platform": platform.platform(),
        "architecture": platform.machine(),
        "source_sha256": digests["source"],
        "binary_sha256": digests["binary"],
        "elapsed_ms": round(elapsed * 1000, 3),
        "exit_code": process.returncode,
        "timed_out": timed_out,
        "primary_error": primary_error,
        "cleanup_errors": cleanup_errors,
        "artifact_errors": artifact_errors,
        "stdout_bytes": len(stdout),
        "stderr_bytes": len(stderr),
        "original_process_waited": process.returncode is not None,
        "parent_limits_unchanged": resource.getrlimit(resource.RLIMIT_NPROC) == before,
        "security_acceptance": False,
        "scope": "Native nonroot process-limit component; not installer, no_new_privs or Engine acceptance",
    }
    # 自有固定探针正常输出仅两条小记录；额外输出一律拒绝，不能当作能力通过。
    record["output_within_record_limit"] = len(stdout) <= 2048 and not stderr
    records = []
    if record["output_within_record_limit"]:
        try:
            records = [json.loads(line) for line in stdout.decode("utf-8").splitlines()]
        except (ValueError, UnicodeError):
            records = []
    expected = [
        {"phase": "before_exec", "nonroot": True, "positive_fork_wait": True,
         "positive_vfork_wait": True, "positive_spawn_wait": True, "fork_errno": errno.EAGAIN, "vfork_errno": errno.EAGAIN,
         "spawn_errno": errno.EAGAIN, "raise_errno": errno.EPERM, "thread_error": 0,
         "join_error": 0, "thread_marker": True},
        {"phase": "after_exec", "nonroot": True, "soft_zero": True, "hard_zero": True},
    ]
    record["observed"] = records
    record["component_qualified"] = (primary_error is None and not cleanup_errors and not artifact_errors
                                      and not timed_out and elapsed < 20 and process.returncode == 0
                                      and record["parent_limits_unchanged"] and records == expected)
    try:
        (output / "result.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    except OSError as error:
        # 结果文件不可写时，控制台仍保留主错及次错；不能输出通过结论。
        artifact_errors.append({"stage": "result", "kind": type(error).__name__, "errno": error.errno})
        record["component_qualified"] = False
    print(json.dumps(record, indent=2))
    if not record["component_qualified"]:
        raise SystemExit("QUALIFICATION_NOT_PROVEN; original result retained")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--source", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--commit", required=True)
    arguments = parser.parse_args()
    qualify(arguments.binary.resolve(), arguments.source.resolve(), arguments.output.resolve(), arguments.commit)
