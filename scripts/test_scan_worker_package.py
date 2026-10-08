#!/usr/bin/env python3
"""同版本 helper 包的组成单元测试；临时字节文件不证明原生执行或安装信任。

PackageCompositionTests 调用真实包装 main，仅替换 accepted 的外部验收流程。
WorkerManifestTests 是未来真实 worker_manifest API 的验收；缺模块/方法只算接口缺失。
链接夹具必须实际创建；权限不足会明确失败，不跳过并冒称链接保护已验收。
"""

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest
from unittest import mock
import zipfile


ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / "scripts"
SUFFIX = ".exe" if sys.platform == "win32" else ""
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
PIN = "158f9cc2f0b332194a3ffc5acec47760c99146d8"
SOURCE_MANIFEST = ROOT / "crates" / "diskgraph-disktree-core" / "UPSTREAM_DIGESTS.txt"
SOURCE_SHA = hashlib.sha256(SOURCE_MANIFEST.read_bytes()).hexdigest()
NAMES = tuple(name + SUFFIX for name in ("diskgraph", "diskgraph-mcp", "diskgraph-scan-worker"))
MANIFEST_NAME = "scan-worker-manifest.json"


def native_target():
    """为组成夹具选择当前平台 target，不执行临时 artifact。"""
    arch = "aarch64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"
    if sys.platform == "win32":
        return arch + "-pc-windows-msvc"
    if sys.platform == "darwin":
        return arch + "-apple-darwin"
    return arch + "-unknown-linux-gnu"


def load_file(test, name, path):
    """按精确文件路径加载实际模块，并在测试结束后恢复模块注册。"""
    spec = importlib.util.spec_from_file_location(name, path)
    test.assertIsNotNone(spec, f"cannot load actual module: {path.name}")
    test.assertIsNotNone(spec.loader)
    module = importlib.util.module_from_spec(spec)
    previous = sys.modules.get(name)
    sys.modules[name] = module

    def restore():
        if previous is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = previous

    test.addCleanup(restore)
    spec.loader.exec_module(module)
    return module


class PackageFixture(unittest.TestCase):
    """真实临时三文件及独立旧 CLI；没有 native 执行或假 ELF/PE 能力判断。"""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="diskgraph-package-unit-")
        self.addCleanup(temporary.cleanup)
        self.work = pathlib.Path(temporary.name)
        self.bin_dir = self.work / "built"
        self.bin_dir.mkdir()
        self.payloads = {
            name: b"composition-only\x00" + name.encode("ascii") + bytes(range(64))
            for name in NAMES
        }
        for name, payload in self.payloads.items():
            (self.bin_dir / name).write_bytes(payload)
        self.old_cli = self.work / ("old-diskgraph" + SUFFIX)
        self.old_cli.write_bytes(b"previous-version-composition-only")
        self.output = self.work / "output"
        self.target = native_target()

    def assert_manifest(self, manifest):
        self.assertEqual(set(manifest), {
            "schema_version", "package_version", "target", "protocol_version",
            "pinned_scanner_revision", "scanner_source_manifest_sha256", "executable",
        })
        self.assertEqual(manifest["schema_version"], 1)
        self.assertEqual(manifest["package_version"], VERSION)
        self.assertEqual(manifest["target"], self.target)
        self.assertEqual(manifest["protocol_version"], 2)
        self.assertEqual(manifest["pinned_scanner_revision"], PIN)
        self.assertEqual(manifest["scanner_source_manifest_sha256"], SOURCE_SHA)
        executable = manifest["executable"]
        self.assertEqual(set(executable), {"name", "bytes", "sha256"})
        self.assertEqual(executable["name"], NAMES[2])
        self.assertEqual(executable["bytes"], len(self.payloads[NAMES[2]]))
        self.assertEqual(executable["sha256"], hashlib.sha256(self.payloads[NAMES[2]]).hexdigest())

    def actual_package(self):
        # 新包装入口可直接 import worker_manifest；从精确文件预加载，避免 PATH 假设。
        path = SCRIPTS / "worker_manifest.py"
        if path.is_file():
            load_file(self, "worker_manifest", path)
        return load_file(self, "diskgraph_package_unit_entry", SCRIPTS / "accept-readonly-package.py")

    def call_main(self, package, accepted):
        arguments = [str(SCRIPTS / "accept-readonly-package.py"), "--target", self.target,
                     "--bin-dir", str(self.bin_dir), "--old-cli", str(self.old_cli),
                     "--output-dir", str(self.output)]
        stdout = io.StringIO()
        # 组成测试的独立预期来自本夹具输入字节，不读取包装生成的清单。
        deployment = {
            "DISKGRAPH_SCAN_WORKER_PATH": str(self.bin_dir / NAMES[2]),
            "DISKGRAPH_SCAN_WORKER_SHA256": hashlib.sha256(self.payloads[NAMES[2]]).hexdigest(),
            "DISKGRAPH_SCAN_WORKER_BYTES": str(len(self.payloads[NAMES[2]])),
        }
        with mock.patch.object(sys, "argv", arguments), mock.patch.object(package, "accepted", accepted):
            with mock.patch.dict(os.environ, deployment), contextlib.redirect_stdout(stdout):
                package.main()
        return json.loads(stdout.getvalue())


class PackageCompositionTests(PackageFixture):
    """旧 main 的 helper 遗漏须是实际组成失败，外部 native 流程由边界替身观察，不执行真实平台验收。"""

    def test_archive_contains_three_exact_artifacts_and_verified_manifest(self):
        package = self.actual_package()
        calls = []

        def accepted(script, bin_dir, *arguments, deployment=None):
            if sys.platform == "linux":
                self.assertEqual(deployment["DISKGRAPH_SCAN_WORKER_PATH"],
                                 str(pathlib.Path(bin_dir) / NAMES[2]))
                self.assertEqual(deployment["DISKGRAPH_SCAN_WORKER_SHA256"],
                                 hashlib.sha256(self.payloads[NAMES[2]]).hexdigest())
            # 观察真实 main 自己解出的内容；不替换 copy、归档、摘要或解压函数。
            files = {path.name: path.read_bytes() for path in pathlib.Path(bin_dir).iterdir() if path.is_file()}
            calls.append((script, files, tuple(map(str, arguments)), str(bin_dir)))
            return 1

        report = self.call_main(package, accepted)
        archive = pathlib.Path(report["archive"])
        self.assertTrue(archive.is_file())
        archive_bytes = archive.read_bytes()
        actual_sha = hashlib.sha256(archive_bytes).hexdigest()
        self.assertEqual(report["sha256"], actual_sha)
        checksum = pathlib.Path(str(archive) + ".sha256").read_text().split()
        self.assertEqual(checksum, [actual_sha, archive.name])
        self.assertEqual(archive.suffix, ".zip" if SUFFIX else ".gz")
        extracted = self.work / "independent-extraction"
        extracted.mkdir()
        if SUFFIX:
            with zipfile.ZipFile(archive) as stream:
                stream.extractall(extracted)
        else:
            with tarfile.open(archive) as stream:
                stream.extractall(extracted, filter="data")
        packaged = extracted / ("diskgraph-" + self.target)
        packaged_bin = packaged / "bin"
        self.assertEqual({path.name for path in packaged_bin.iterdir()}, set(NAMES) | {MANIFEST_NAME})
        for name, payload in self.payloads.items():
            self.assertEqual((packaged_bin / name).read_bytes(), payload)
            self.assertEqual((self.bin_dir / name).read_bytes(), payload)
        manifest = json.loads((packaged_bin / MANIFEST_NAME).read_bytes())
        self.assert_manifest(manifest)
        for name in ("README.md", "LICENSE"):
            self.assertEqual((packaged / name).read_bytes(), (ROOT / name).read_bytes())
        self.assertEqual([call[0] for call in calls], [
            "accept-readonly-stdio.py", "accept-readonly-http.py",
            "accept-readonly-upgrade.py", "accept-readonly-load.py",
            "accept-readonly-load.py",
        ])
        for _, files, _, _ in calls:
            self.assertEqual(set(files), set(NAMES) | {MANIFEST_NAME})
            for name, payload in self.payloads.items():
                self.assertEqual(files[name], payload)
            self.assert_manifest(json.loads(files[MANIFEST_NAME]))
        self.assertIn(str(self.old_cli.resolve()), calls[2][2])
        http = dict(zip(calls[1][2][::2], calls[1][2][1::2]))
        self.assertEqual(http['--soak-seconds'], '60')
        self.assertTrue(http['--output'].endswith('.http.json'))
        small = dict(zip(calls[3][2][::2], calls[3][2][1::2]))
        large = dict(zip(calls[4][2][::2], calls[4][2][1::2]))
        self.assertEqual(small["--files"], "20000")
        self.assertEqual(large["--files"], "200000")
        self.assertEqual(small["--bin-dir"], calls[3][3])
        self.assertEqual(large["--bin-dir"], calls[4][3])
        self.assertEqual(calls[3][3], calls[4][3])
        self.assertNotEqual(small["--output"], large["--output"])
        self.assertTrue(small["--output"].endswith(".load.json"))
        self.assertTrue(large["--output"].endswith(".load_200000.json"))
        for field in ("stdio", "http", "upgrade_rollback", "controlled_load",
                      "controlled_load_200000"):
            self.assertEqual(report[field], 1)

    def test_missing_helper_fails_before_any_external_acceptance(self):
        package = self.actual_package()
        (self.bin_dir / NAMES[2]).unlink()
        calls = []

        def accepted(script, bin_dir, *arguments):
            calls.append(script)
            return 1

        with self.assertRaises((FileNotFoundError, ValueError, RuntimeError)) as raised:
            self.call_main(package, accepted)
        self.assertIn("diskgraph-scan-worker", str(raised.exception))
        self.assertEqual(calls, [], "missing helper reached external acceptance")


class WorkerManifestTests(PackageFixture):
    """真实 create/verify API 合同；模块或方法缺失明确为 API absence。"""

    def setUp(self):
        super().setUp()
        path = SCRIPTS / "worker_manifest.py"
        self.assertTrue(path.is_file(), "API absence: scripts/worker_manifest.py does not exist; not runtime RED")
        self.api = load_file(self, "worker_manifest", path)
        for name in ("create_manifest", "verify_manifest"):
            self.assertTrue(callable(getattr(self.api, name, None)), f"API absence: {name}; not runtime RED")

    def create(self):
        return self.api.create_manifest(self.bin_dir, self.target, VERSION)

    def verify(self, manifest):
        self.api.verify_manifest(self.bin_dir, manifest, self.target, VERSION)

    def rejected(self, manifest):
        with self.assertRaises((ValueError, OSError)):
            self.verify(manifest)

    def test_create_and_verify_real_native_suffix_size_digest_and_source_pin(self):
        manifest = self.create()
        self.assert_manifest(manifest)
        self.verify(manifest)

    def test_unknown_top_level_and_nested_fields_are_rejected(self):
        original = self.create()
        for nested in (False, True):
            with self.subTest(nested=nested):
                manifest = copy.deepcopy(original)
                (manifest["executable"] if nested else manifest)["unknown"] = "forbidden"
                self.rejected(manifest)

    def test_wrong_version_target_protocol_pin_and_source_manifest_are_rejected(self):
        original = self.create()
        changes = {
            "schema_version": 2, "package_version": VERSION + "-wrong",
            "target": self.target + "-wrong", "protocol_version": 1,
            "pinned_scanner_revision": "0" * 40,
            "scanner_source_manifest_sha256": "0" * 64,
        }
        for field, value in changes.items():
            with self.subTest(field=field):
                manifest = copy.deepcopy(original)
                manifest[field] = value
                self.rejected(manifest)

    def test_wrong_executable_basename_size_or_digest_are_rejected(self):
        original = self.create()
        changes = (("name", "../" + NAMES[2]), ("name", NAMES[0]),
                   ("bytes", original["executable"]["bytes"] + 1), ("sha256", "0" * 64))
        for field, value in changes:
            with self.subTest(field=field, value=value):
                manifest = copy.deepcopy(original)
                manifest["executable"][field] = value
                self.rejected(manifest)

    def test_same_size_byte_change_invalidates_original_manifest(self):
        manifest = self.create()
        path = self.bin_dir / NAMES[2]
        original = path.read_bytes()
        changed = bytes([original[0] ^ 1]) + original[1:]
        self.assertEqual(len(changed), len(original))
        path.write_bytes(changed)
        self.rejected(manifest)

    def test_missing_or_directory_helper_is_rejected_by_create_and_verify(self):
        manifest = self.create()
        path = self.bin_dir / NAMES[2]
        path.unlink()
        for directory in (False, True):
            with self.subTest(directory=directory):
                if directory:
                    path.mkdir()
                with self.assertRaises((ValueError, OSError)):
                    self.create()
                self.rejected(manifest)

    def test_helper_symlink_is_rejected_without_skipping_fixture_qualification(self):
        manifest = self.create()
        path = self.bin_dir / NAMES[2]
        target = self.work / "actual-helper-bytes"
        path.rename(target)
        try:
            path.symlink_to(target)
        except OSError as error:
            self.fail(f"symlink fixture qualification unavailable ({error.__class__.__name__}); protection not accepted")
        self.assertTrue(path.is_symlink())
        with self.assertRaises((ValueError, OSError)):
            self.create()
        self.rejected(manifest)




class TimeoutDiagnosticTests(PackageFixture):
    """外层超时保留原异常并输出已捕获的阶段诊断。"""

    def test_timeout_replays_partial_stderr_and_preserves_exception(self):
        package = self.actual_package()
        failure = subprocess.TimeoutExpired(['acceptance'], 300,
                                            output=b'partial stdout',
                                            stderr=b'phase=index begin\n')
        diagnostics = io.StringIO()
        with mock.patch.object(package, 'run_acceptance', side_effect=failure), \
                contextlib.redirect_stderr(diagnostics):
            with self.assertRaises(subprocess.TimeoutExpired) as caught:
                package.accepted('accept-readonly-load.py', self.bin_dir)
        self.assertIs(caught.exception, failure)
        self.assertIn('phase=index begin', diagnostics.getvalue())
        self.assertIn('partial stdout', diagnostics.getvalue())

    def test_failure_retains_workspace_and_original_exception(self):
        package = self.actual_package()
        failure = subprocess.TimeoutExpired(['acceptance'], 300)
        retained = []

        def fail(script, bin_dir, *arguments, **kwargs):
            retained.append(pathlib.Path(bin_dir).parents[2])
            raise failure

        diagnostics = io.StringIO()
        try:
            with contextlib.redirect_stderr(diagnostics):
                with self.assertRaises(subprocess.TimeoutExpired) as caught:
                    self.call_main(package, fail)
            self.assertIs(caught.exception, failure)
            self.assertTrue(retained[0].exists())
            self.assertIn(str(retained[0]), diagnostics.getvalue())
        finally:
            for path in retained:
                # 这里没有真实子进程，测试拥有全部临时文件。
                shutil.rmtree(path, ignore_errors=True)

if __name__ == "__main__":
    unittest.main()
