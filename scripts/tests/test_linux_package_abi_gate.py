"""实际包入口在写归档前拒绝不符合声明基线的ELF；夹具不代表可执行程序。"""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
from test_linux_abi_requirements import fixture

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location("package_abi_gate", SCRIPTS / "accept-readonly-package.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)
import linux_abi_requirements as ABI

class LinuxPackageAbiGateTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("diskgraph", "diskgraph-mcp", "diskgraph-scan-worker"):
            (self.bin / name).write_bytes(fixture(["GLIBC_2.17"], 62))

    def test_package_entry_refuses_high_abi_before_archive_or_subprocess(self):
        for name in ("diskgraph", "diskgraph-mcp", "diskgraph-scan-worker"):
            with self.subTest(name=name):
                image = self.bin / name
                image.write_bytes(fixture(["GLIBC_2.39"], 62))
                output = self.root / "output"
                output.mkdir(exist_ok=True)
                old_archive = output / "diskgraph-x86_64-unknown-linux-gnu.tar.gz"
                old_archive.write_bytes(b"existing archive must survive")
                argv = ["accept-readonly-package.py", "--target", "x86_64-unknown-linux-gnu",
                        "--bin-dir", str(self.bin), "--old-cli", str(self.root / "old"),
                        "--output-dir", str(output)]
                with mock.patch.object(sys, "argv", argv), mock.patch.object(PACKAGE, "accepted", return_value=0) as run:
                    with self.assertRaisesRegex(ValueError, "2.39"):
                        PACKAGE.main()
                    run.assert_not_called()
                self.assertEqual(old_archive.read_bytes(), b"existing archive must survive")
                self.assertEqual(list(output.iterdir()), [old_archive])
                image.write_bytes(fixture(["GLIBC_2.17"], 62))

    def test_baseline_report_binds_all_actual_image_digests(self):
        report = ABI.require_gnu_package(self.bin, "x86_64-unknown-linux-gnu")
        self.assertEqual(report["advertised_baseline"], "2.17")
        self.assertEqual(len(report["images"]), 3)
        for row in report["images"]:
            self.assertEqual(row["glibc_requirements"], ["2.17"])
            self.assertEqual(len(row["sha256"]), 64)
            self.assertEqual(row["bytes"], (self.bin / row["name"]).stat().st_size)
        self.assertFalse(report["runtime_qualification"])

    def test_target_architecture_and_nonregular_image_are_refused(self):
        with self.assertRaisesRegex(ValueError, "architecture"):
            ABI.require_gnu_package(self.bin, "aarch64-unknown-linux-gnu")
        (self.bin / "diskgraph").unlink()
        (self.bin / "diskgraph").mkdir()
        with self.assertRaises((ValueError, OSError)):
            ABI.require_gnu_package(self.bin, "x86_64-unknown-linux-gnu")

    def test_private_bundle_checks_built_images_before_install_or_manifest(self):
        script = (SCRIPTS / "package-linux.sh").read_text()
        gate = script.index("scripts/linux_abi_requirements.py")
        self.assertLess(script.index("cargo build --release --locked"), gate)
        self.assertLess(gate, script.index("install -m 0755"))
        self.assertIn("--maximum-glibc 2.17", script)

if __name__ == "__main__":
    unittest.main()
