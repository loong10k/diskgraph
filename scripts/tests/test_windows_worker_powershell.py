"""Windows PowerShell 5.1 原生验收脚本回归；合成镜像只验证打包规则。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/configure_windows_acceptance_worker.ps1"
EXIT_SCRIPT = ROOT / "scripts/configure_windows_exit_fixtures.ps1"


@unittest.skipUnless(os.name == "nt", "requires actual Windows PowerShell")
class WindowsWorkerPowerShellTests(unittest.TestCase):
    def test_native_exit_fixture_build_does_not_depend_on_active_code_page(self):
        with tempfile.TemporaryDirectory(prefix="dg-exit-ps51-") as directory:
            root = Path(directory)
            environment = dict(os.environ, RUNNER_TEMP=str(root), GITHUB_ENV=str(root / "acceptance.env"))
            environment.pop("CL", None)
            environment.pop("_CL_", None)
            result = subprocess.run([
                "powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                "-File", str(EXIT_SCRIPT),
            ], cwd=ROOT, env=environment, capture_output=True, timeout=90)
            self.assertEqual(result.returncode, 0, (result.stdout + result.stderr).decode(errors="replace"))
            receipt = json.loads((root / "diskgraph-windows-exit-fixtures/receipt.json").read_text(encoding="utf-8-sig"))
            exported = dict(line.split("=", 1) for line in (root / "acceptance.env").read_text(encoding="utf-8-sig").splitlines())
            self.assertEqual({r["code"] for r in receipt["artifacts"]}, {"C0000000", "C0000005"})
            for artifact in receipt["artifacts"]:
                binary = Path(artifact["path"])
                self.assertEqual(exported["DISKGRAPH_WINDOWS_EXIT_" + artifact["code"] + "_FIXTURE"], str(binary))
                self.assertEqual(hashlib.sha256(binary.read_bytes()).hexdigest(), artifact["sha256"])
                exited = subprocess.run([str(binary)], capture_output=True, timeout=10)
                self.assertEqual(exited.returncode & 0xFFFFFFFF, int(artifact["code"], 16))

    def run_fixture(self, source, output, artifacts):
        artifacts.write_text(json.dumps({
            "reason": "compiler-artifact",
            "target": {"name": "diskgraph-scan-worker", "kind": ["bin"]},
            "executable": str(source),
        }) + "\n", encoding="utf-8")
        environment = dict(os.environ, GITHUB_ENV=str(output / "acceptance.env"))
        return subprocess.run([
            "powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", str(SCRIPT),
            "-Artifacts", str(artifacts), "-OutputDir", str(output),
        ], cwd=ROOT, env=environment, capture_output=True, timeout=30)

    def test_windows_powershell_copies_exact_bytes_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory(prefix="dg-worker-ps51-") as directory:
            root = Path(directory)
            source = root / "synthetic.exe"
            payload = b"test fixture only, not an executable\x00\xff"
            source.write_bytes(payload)
            result = self.run_fixture(source, root, root / "artifacts.jsonl")
            self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
            receipt = json.loads((root / "product-worker-receipt.json").read_text(encoding="utf-8-sig"))
            self.assertEqual(receipt["worker_sha256"].lower(), hashlib.sha256(payload).hexdigest())
            self.assertEqual(receipt["worker_bytes"], len(payload))
            self.assertEqual((root / "product-scan-worker.exe").read_bytes(), payload)
            before = (root / "acceptance.env").read_bytes()
            source.write_bytes(b"replacement")
            result = self.run_fixture(source, root, root / "artifacts.jsonl")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((root / "product-scan-worker.exe").read_bytes(), payload)
            self.assertEqual((root / "acceptance.env").read_bytes(), before)

    def test_relative_drive_relative_and_root_relative_are_rejected(self):
        with tempfile.TemporaryDirectory(prefix="dg-worker-paths-") as directory:
            root = Path(directory)
            for source in ["relative.exe", "C:relative.exe", "\\relative.exe"]:
                with self.subTest(source=source):
                    result = self.run_fixture(source, root, root / "artifacts.jsonl")
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse((root / "product-scan-worker.exe").exists())
                    self.assertFalse((root / "acceptance.env").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
