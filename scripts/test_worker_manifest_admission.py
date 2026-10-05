#!/usr/bin/env python3
"""安装材料的真实文件准入反例；不证明运行时镜像信任。"""

import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import worker_manifest as manifest
from test_scan_worker_package import NAMES, PackageFixture


class PackageSourceAdmissionTests(PackageFixture):
    """真实包装入口不能把源链接复制成普通文件后当作合格材料。"""

    def test_actual_package_rejects_linked_source_before_external_acceptance(self):
        package = self.actual_package()
        helper = self.bin_dir / NAMES[2]
        original = helper.with_suffix(".actual")
        helper.rename(original)
        helper.symlink_to(original)
        calls = []
        with self.assertRaises((ValueError, OSError)):
            self.call_main(package, lambda *args: calls.append(args) or 1)
        self.assertEqual(calls, [])


class ManifestAdmissionTests(unittest.TestCase):
    """用真实路径替换验证两种读取都不接受检查后的新对象。"""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="worker-admission-")
        self.addCleanup(temporary.cleanup)
        self.path = Path(temporary.name) / "artifact"
        self.path.write_bytes(b'{"schema_version": 1}')

    def test_same_path_replacement_is_rejected_for_manifest_and_image(self):
        real_open = os.open
        for reader in (manifest.read_manifest, manifest.image_metadata):
            self.path.write_bytes(b'{"schema_version": 1}')

            def replace_then_open(path, flags, *args, **kwargs):
                replacement = self.path.with_suffix(".replacement")
                replacement.write_bytes(b'{"schema_version": 2}')
                replacement.replace(self.path)
                return real_open(path, flags, *args, **kwargs)

            with self.subTest(reader=reader.__name__):
                with mock.patch.object(manifest.os, "open", replace_then_open):
                    with self.assertRaises(ValueError):
                        reader(self.path)

    @unittest.skipIf(os.name == "nt", "FIFO is a Unix-specific admission case")
    def test_fifo_replacement_uses_nonblocking_open_and_rejects_object(self):
        real_open = os.open
        for reader in (manifest.read_manifest, manifest.image_metadata):
            self.path.unlink(missing_ok=True)
            self.path.write_bytes(b'{"schema_version": 1}')

            def replace_then_open(path, flags, *args, **kwargs):
                self.assertTrue(flags & os.O_NONBLOCK, "FIFO admission can block before fstat")
                self.path.unlink()
                os.mkfifo(self.path)
                return real_open(path, flags, *args, **kwargs)

            with self.subTest(reader=reader.__name__):
                with mock.patch.object(manifest.os, "open", replace_then_open):
                    with self.assertRaises(ValueError):
                        reader(self.path)

    def test_manifest_rejects_oversize_and_duplicate_fields(self):
        self.path.write_bytes(b" " * (manifest.MAX_MANIFEST_BYTES + 1))
        with self.assertRaises(ValueError):
            manifest.read_manifest(self.path)
        self.path.write_bytes(b'{"schema_version": 1, "schema_version": 2}')
        with self.assertRaises(ValueError):
            manifest.read_manifest(self.path)
        self.path.write_text(json.dumps({"schema_version": 1}))
        self.assertEqual(manifest.read_manifest(self.path), {"schema_version": 1})


if __name__ == "__main__":
    unittest.main()
