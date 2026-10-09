#!/usr/bin/env python3
"""Package native binaries, verify the archive, and drill read-only upgrade/rollback."""

import argparse
import contextlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from linux_abi_requirements import GNU_TARGET_MACHINES, require_gnu_package
from worker_manifest import (MANIFEST_NAME, MAX_IMAGE_BYTES, bounded_digest,
                             copy_artifact, executable_name, image_metadata, read_manifest,
                             verify_manifest, workspace_version, write_manifest)


if sys.platform == "win32":
    from windows_acceptance_job import run_owned as run_acceptance
else:
    run_acceptance = subprocess.run


ROOT = pathlib.Path(__file__).resolve().parent.parent
SUFFIX = ".exe" if sys.platform == "win32" else ""


def digest(path, byte_limit=MAX_IMAGE_BYTES):
    return bounded_digest(path, byte_limit)


def packaged_worker_environment(bin_dir, inherited):
    """用受控构建的独立预期核验包内镜像；清单不得自行提供执行信任。"""
    keys = ('PATH', 'SHA256', 'BYTES')
    values = [inherited.get('DISKGRAPH_SCAN_WORKER_' + key) for key in keys]
    if not all(isinstance(value, str) and value for value in values):
        raise ValueError('package acceptance requires independent build deployment values')
    source, expected_digest, expected_bytes = values
    if not pathlib.Path(source).is_absolute() or any(char in source for char in '\r\n\0'):
        raise ValueError('invalid controlled build deployment path')
    if not re.fullmatch('[0-9a-f]{64}', expected_digest) or not re.fullmatch('[0-9]+', expected_bytes):
        raise ValueError('invalid controlled build deployment digest or length')
    length = int(expected_bytes)
    if not 0 < length <= MAX_IMAGE_BYTES:
        raise ValueError('controlled build deployment exceeds image budget')
    worker = pathlib.Path(bin_dir).absolute() / 'diskgraph-scan-worker'
    metadata = image_metadata(worker)
    if metadata['sha256'] != expected_digest or metadata['bytes'] != length:
        raise ValueError('packaged worker differs from independent build deployment')
    environment = dict(inherited)
    # 执行解包后的同字节镜像；Rust 宿主继续核验并持有实际执行镜像。
    environment['DISKGRAPH_SCAN_WORKER_PATH'] = str(worker)
    return environment


def accepted(script, bin_dir, *arguments, deployment=None):
    environment = dict(os.environ if deployment is None else deployment)
    environment["DISKGRAPH_ACCEPT_BIN_DIR"] = str(bin_dir)
    # 验收子脚本输出固定 UTF-8；宿主代码页不决定 JSON/诊断协议编码。
    environment["PYTHONIOENCODING"] = "utf-8"
    try:
        result = run_acceptance(
            [sys.executable, str(ROOT / "scripts" / script), *map(str, arguments)],
            cwd=ROOT, env=environment, capture_output=True, text=True, encoding="utf-8", timeout=300,
        )
    except subprocess.TimeoutExpired as failure:
        # TimeoutExpired 即使 text=True 也可能携带 bytes；保留阶段日志及原异常。
        for captured in (failure.stdout, failure.stderr):
            if captured:
                diagnostic = (captured.decode("utf-8", errors="replace")
                              if isinstance(captured, bytes) else captured)
                print(diagnostic, file=sys.stderr, flush=True)
        raise
    if result.returncode:
        raise RuntimeError(f"{script} failed:\n{result.stdout}\n{result.stderr}")
    report = json.loads(result.stdout)
    if report["passed"] != report["total"]:
        raise RuntimeError(f"{script} did not pass every check: {result.stdout}")
    return report["passed"]


@contextlib.contextmanager
def acceptance_workspace():
    """成功后回收；失败时保留可能仍被后代使用的文件并传播原异常。"""
    temporary = tempfile.mkdtemp(prefix="diskgraph-package-accept-")
    try:
        yield temporary
    except BaseException:
        print(f"package acceptance failed; retained workspace: {temporary}",
              file=sys.stderr, flush=True)
        raise
    else:
        shutil.rmtree(temporary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--bin-dir", required=True, type=pathlib.Path)
    parser.add_argument("--old-cli", required=True, type=pathlib.Path)
    parser.add_argument("--output-dir", default=ROOT / "dist", type=pathlib.Path)
    args = parser.parse_args()
    executable_name(args.target)
    binaries = [f"diskgraph{SUFFIX}", f"diskgraph-mcp{SUFFIX}", f"diskgraph-scan-worker{SUFFIX}"]
    source = args.bin_dir.resolve()
    # 在写归档或执行程序之前拒绝不满足原兼容声明的真实GNU制品。
    if args.target in GNU_TARGET_MACHINES:
        require_gnu_package(source, args.target)
    old_cli = args.old_cli.resolve()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    archive_name = f"diskgraph-{args.target}"
    archive = output / (archive_name + (".zip" if SUFFIX else ".tar.gz"))

    with acceptance_workspace() as temporary:
        work = pathlib.Path(temporary)
        staging = work / archive_name
        staging_bin = staging / "bin"
        staging_bin.mkdir(parents=True)
        for name in binaries:
            copy_artifact(source / name, staging_bin / name)
        if args.target in GNU_TARGET_MACHINES:
            abi = require_gnu_package(staging_bin, args.target)
            (staging / "gnu-abi.json").write_text(json.dumps(abi, indent=2) + "\n")
        version = workspace_version()
        write_manifest(staging_bin, args.target, version)
        for name in ("README.md", "LICENSE"):
            shutil.copy2(ROOT / name, staging / name)

        if SUFFIX:
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as package:
                for file in staging.rglob("*"):
                    if file.is_file():
                        package.write(file, file.relative_to(work))
        else:
            with tarfile.open(archive, "w:gz") as package:
                package.add(staging, arcname=archive_name)
        archive_hash = digest(archive, 1024 * 1024 * 1024)
        checksum = pathlib.Path(str(archive) + ".sha256")
        checksum.write_text(f"{archive_hash}  {archive.name}\n")
        if checksum.read_text().split()[0] != digest(archive, 1024 * 1024 * 1024):
            raise RuntimeError("archive checksum changed before extraction")

        extracted = work / "extracted"
        extracted.mkdir()
        if SUFFIX:
            with zipfile.ZipFile(archive) as package:
                package.extractall(extracted)
        else:
            with tarfile.open(archive) as package:
                package.extractall(extracted, filter="data")
        packaged_bin = extracted / archive_name / "bin"
        for name in binaries:
            if digest(source / name) != digest(packaged_bin / name):
                raise RuntimeError(f"packaged {name} differs from the built binary")
        manifest = read_manifest(packaged_bin / MANIFEST_NAME)
        verify_manifest(packaged_bin, manifest, args.target, version)
        if args.target in GNU_TARGET_MACHINES:
            if require_gnu_package(packaged_bin, args.target) != abi:
                raise RuntimeError("extracted GNU ABI differs from staged images")

        deployment = (packaged_worker_environment(packaged_bin, os.environ)
                      if sys.platform == "linux" else None)
        stdio = accepted("accept-readonly-stdio.py", packaged_bin, deployment=deployment)
        http = accepted("accept-readonly-http.py", packaged_bin,
                        "--soak-seconds", "60",
                        "--output", output / f"{archive_name}.http.json",
                        deployment=deployment)
        upgrade = accepted(
            "accept-readonly-upgrade.py", packaged_bin,
            "--old-cli", old_cli, "--new-cli", packaged_bin / binaries[0],
            deployment=deployment,
        )
        load = accepted(
            "accept-readonly-load.py", packaged_bin,
            "--bin-dir", packaged_bin, "--files", "20000",
            "--output", output / f"{archive_name}.load.json",
            deployment=deployment,
        )
        large_load = accepted(
            "accept-readonly-load.py", packaged_bin,
            "--bin-dir", packaged_bin, "--files", "200000",
            "--output", output / f"{archive_name}.load_200000.json",
            deployment=deployment,
        )
    print(json.dumps({"target": args.target, "archive": str(archive),
                      "sha256": archive_hash, "stdio": stdio,
                      "http": http, "upgrade_rollback": upgrade,
                      "controlled_load": load, "controlled_load_200000": large_load, "scan_worker": manifest}, indent=2))


if __name__ == "__main__":
    main()
