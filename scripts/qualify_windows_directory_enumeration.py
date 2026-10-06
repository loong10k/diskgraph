"""只替换确切已提交的旧目录枚举实现，要求实际行为目标RED；不接受编译失败。"""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess

import qualify_macos_installed_worker as shared
from qualify_windows_prepared_birth_baseline import isolated_source
from qualify_windows_scan_cleanup import CANDIDATE, check_platform

OLD_COMMIT = "888fbc20031baed5cca113eacf18e7ae81590ecd"
SOURCE = "crates/diskgraph-engine/src/live_evidence/git_directory_lease.rs"
CASE = "live_evidence::windows_git_directory_cursor_tests::held_directory_names_ignore_foreign_path_argument"


def check_target_red(stdout, stderr):
    if ("test " + CASE + " ... " not in stdout
            or "test result: FAILED. 0 passed; 1 failed; 0 ignored;" not in stdout
            or "DG_HELD_DIRECTORY_ARGUMENT_CONTROL=1" not in stdout
            or "held directory must never enumerate the foreign argument path" not in stderr):
        raise RuntimeError("old directory enumeration did not fail at the exact held-object assertion")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    checkout = Path(__file__).resolve().parent.parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    receipt = {"production_acceptance": False, "status": "started", "case": CASE}
    try:
        check_platform(platform.system(), os.environ.get("GITHUB_ACTIONS"), os.environ.get("RUNNER_ENVIRONMENT"))
        original = subprocess.check_output(["git", "show", OLD_COMMIT + ":" + SOURCE], cwd=checkout)
        with isolated_source(checkout, CANDIDATE) as (source, manifest):
            receipt["checkout_sha"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
            receipt["candidate"] = manifest
            receipt["old_commit"] = OLD_COMMIT
            path = source / SOURCE
            path.write_bytes(original)
            receipt["old_source_sha256"] = shared.digest(path)
            environment = os.environ.copy()
            environment["CARGO_TARGET_DIR"] = str(output.parent / "windows-qualification-target")
            shared.invoke(["cargo", "test", "--locked", "-p", "diskgraph-engine", "--lib",
                           "--no-run", "--message-format=json"], source, output, "build-held-directory-red", environment)
            binaries = []
            for line in (output / "build-held-directory-red.stdout").read_text().splitlines():
                if line.startswith("{"):
                    record = json.loads(line)
                    if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "diskgraph_engine" and record.get("executable"):
                        binaries.append(Path(record["executable"]))
            if len(binaries) != 1:
                raise RuntimeError("expected one actual directory baseline test executable")
            binary = binaries[0]
            receipt["fixture_sha256"] = shared.digest(binary)
            try:
                shared.invoke([str(binary), "--exact", CASE, "--nocapture", "--test-threads=1"],
                              source, output, "held-directory-red", environment, timeout=120)
            except RuntimeError:
                check_target_red((output / "held-directory-red.stdout").read_text(),
                                 (output / "held-directory-red.stderr").read_text())
            else:
                raise RuntimeError("old directory enumeration unexpectedly passed")
            for name, expected in manifest["sources"].items():
                actual = shared.digest(source / name)
                if actual != (receipt["old_source_sha256"] if name == SOURCE else expected):
                    raise RuntimeError("directory baseline changed another frozen source")
            if shared.digest(binary) != receipt["fixture_sha256"]:
                raise RuntimeError("directory baseline binary identity changed")
            receipt["status"] = "verified_target_red"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
