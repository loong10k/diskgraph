#!/usr/bin/env python3
"""PF-01: compile and execute Kotlin/JVM against this host's native Rust library."""
import base64
import hashlib
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
JNA_SHA = "b3a9408e7c51e08ef0e3bfcc08f443f6ec0f6191ba8cd7c18d53d2b22e5bdbc0"


def run(arguments, cwd):
    subprocess.run(arguments, cwd=cwd, check=True, timeout=600)


def main():
    maven = shutil.which("mvn")
    java = shutil.which("java")
    if not maven or not java:
        raise RuntimeError("Kotlin host acceptance requires existing Maven and Java; nothing is installed automatically")
    profile = os.environ.get("DISKGRAPH_FFI_PROFILE", "release")
    if profile not in ("debug", "release"):
        raise RuntimeError("DISKGRAPH_FFI_PROFILE must be debug or release")
    build = ["cargo", "build", "-p", "diskgraph-ffi", "--locked"]
    if profile == "release":
        build.append("--release")
    run(build, ROOT)
    suffix = ".exe" if sys.platform == "win32" else ""
    library_name = {
        "darwin": "libdiskgraph_ffi.dylib",
        "win32": "diskgraph_ffi.dll",
        "linux": "libdiskgraph_ffi.so",
    }.get(sys.platform)
    if library_name is None:
        raise RuntimeError(f"unverified Kotlin host platform: {sys.platform}")
    library_directory = ROOT / "target" / profile
    with tempfile.TemporaryDirectory(prefix="diskgraph-kotlin-") as temporary:
        work = pathlib.Path(temporary)
        project = work / "host"
        shutil.copytree(ROOT / "fixtures" / "ffi_kotlin_host", project)
        source = project / "src" / "main" / "kotlin"
        source.mkdir(parents=True)
        run([str(library_directory / f"uniffi-bindgen{suffix}"), "generate",
             "--library", str(library_directory / library_name), "--language", "kotlin",
             "--no-format", "--out-dir", str(source)], ROOT)
        shutil.copyfile(ROOT / "scripts" / "ffi-smoke-host.kt", source / "KotlinHost.kt")
        root = work / "文件 é 空格"
        data = work / "data"
        root.mkdir()
        data.mkdir()
        (root / "README.md").write_text("host fixture\n", encoding="utf-8")
        (root / "blob.bin").write_bytes(b"x" * 65536)
        (root / "文件-é-ß.txt").write_text("Unicode fixture\n", encoding="utf-8")
        classpath = work / "runtime-classpath.txt"
        settings = str(project / "maven-settings.xml")
        run([maven, "--batch-mode", "--errors", "--no-transfer-progress", "--settings", settings,
             "--global-settings", settings, f"-Dmaven.repo.local={ROOT / 'target' / 'ffi-kotlin-m2'}", "compile",
             "org.apache.maven.plugins:maven-dependency-plugin:3.10.0:build-classpath",
             f"-Dmdep.outputFile={classpath}"], project)
        jars = classpath.read_text(encoding="utf-8-sig").strip().split(os.pathsep)
        jna = [pathlib.Path(jar) for jar in jars if pathlib.Path(jar).name == "jna-5.17.0.jar"]
        if len(jna) != 1 or hashlib.sha256(jna[0].read_bytes()).hexdigest() != JNA_SHA:
            raise RuntimeError("JNA 5.17.0 runtime digest mismatch")
        runtime = os.pathsep.join([str(project / "target" / "classes"), *jars])
        # Windows JDK 21 的启动参数会经过本地 ANSI code page；用 ASCII 传输，
        # 宿主再恢复真正的 Unicode String，仍以原 Unicode 路径调用 Rust FFI。
        paths = [base64.b64encode(str(path).encode("utf-8")).decode("ascii")
                 for path in (root, data / "graph.sqlite")]
        run([java, f"-Djna.library.path={library_directory}", "-Djna.encoding=UTF-8",
             "-cp", runtime, "KotlinHostKt", *paths, "4", "utf8-base64"], project)
    print("Native Kotlin/JVM acceptance passed; temporary graph/control files were cleaned")


if __name__ == "__main__":
    main()
