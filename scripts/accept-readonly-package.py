#!/usr/bin/env python3
"""Package native binaries, verify the archive, and drill read-only upgrade/rollback."""

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile


ROOT = pathlib.Path(__file__).resolve().parent.parent
SUFFIX = ".exe" if sys.platform == "win32" else ""


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def accepted(script, bin_dir, *arguments):
    environment = os.environ.copy()
    environment["DISKGRAPH_ACCEPT_BIN_DIR"] = str(bin_dir)
    result = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / script), *map(str, arguments)],
        cwd=ROOT, env=environment, capture_output=True, text=True, timeout=300,
    )
    if result.returncode:
        raise RuntimeError(f"{script} failed:\n{result.stdout}\n{result.stderr}")
    report = json.loads(result.stdout)
    if report["passed"] != report["total"]:
        raise RuntimeError(f"{script} did not pass every check: {result.stdout}")
    return report["passed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--bin-dir", required=True, type=pathlib.Path)
    parser.add_argument("--old-cli", required=True, type=pathlib.Path)
    parser.add_argument("--output-dir", default=ROOT / "dist", type=pathlib.Path)
    args = parser.parse_args()
    binaries = [f"diskgraph{SUFFIX}", f"diskgraph-mcp{SUFFIX}"]
    source = args.bin_dir.resolve()
    old_cli = args.old_cli.resolve()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    archive_name = f"diskgraph-{args.target}"
    archive = output / (archive_name + (".zip" if SUFFIX else ".tar.gz"))

    with tempfile.TemporaryDirectory(prefix="diskgraph-package-accept-") as temporary:
        work = pathlib.Path(temporary)
        staging = work / archive_name
        staging_bin = staging / "bin"
        staging_bin.mkdir(parents=True)
        for name in binaries:
            shutil.copy2(source / name, staging_bin / name)
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
        archive_hash = digest(archive)
        checksum = pathlib.Path(str(archive) + ".sha256")
        checksum.write_text(f"{archive_hash}  {archive.name}\n")
        if checksum.read_text().split()[0] != digest(archive):
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

        stdio = accepted("accept-readonly-stdio.py", packaged_bin)
        http = accepted("accept-readonly-http.py", packaged_bin)
        upgrade = accepted(
            "accept-readonly-upgrade.py", packaged_bin,
            "--old-cli", old_cli, "--new-cli", packaged_bin / binaries[0],
        )
        load = accepted(
            "accept-readonly-load.py", packaged_bin,
            "--bin-dir", packaged_bin,
            "--output", output / f"{archive_name}.load.json",
        )
    print(json.dumps({"target": args.target, "archive": str(archive),
                      "sha256": archive_hash, "stdio": stdio,
                      "http": http, "upgrade_rollback": upgrade,
                      "controlled_load": load}, indent=2))


if __name__ == "__main__":
    main()
