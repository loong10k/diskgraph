"""GNU ABI 命令失败须绑定实际产物，不输出可误用的部分成功证据。"""
import json
import contextlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from test_linux_abi_requirements import fixture
import linux_abi_requirements as ABI


class GnuAbiCliTests(unittest.TestCase):
    """以真实有限 ELF 文件调用命令入口，不执行 ELF 或模拟检查器。"""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.script = Path(__file__).resolve().parents[1] / "linux_abi_requirements.py"

    def run_gate(self, paths):
        """参数为实际镜像路径；返回原 Python 子进程的完整输出和退出状态。"""
        return subprocess.run(
            [sys.executable, str(self.script), "--maximum-glibc", "2.17",
             "--target", "x86_64-unknown-linux-gnu", *map(str, paths)],
            capture_output=True, text=True, check=False,
        )

    def assert_refusal(self, result, path, detail):
        """确认拒绝诊断与当前目标绑定，而且不存在部分成功输出。"""
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        report = json.loads(result.stderr)
        self.assertFalse(report["ok"])
        self.assertEqual(report["error"], "gnu_abi_refused")
        self.assertEqual(report["image"], str(path))
        self.assertEqual(report["target"], "x86_64-unknown-linux-gnu")
        self.assertEqual(report["advertised_baseline"], "2.17")
        self.assertIn(detail, report["detail"])

    def test_later_high_abi_image_does_not_emit_earlier_success(self):
        first = self.root / "diskgraph"
        rejected = self.root / "diskgraph-mcp"
        first.write_bytes(fixture(["GLIBC_2.17"], 62))
        rejected.write_bytes(fixture(["GLIBC_2.39"], 62))
        self.assert_refusal(self.run_gate([first, rejected]), rejected, "2.39")

    def test_wrong_target_image_has_bound_refusal(self):
        rejected = self.root / "diskgraph"
        rejected.write_bytes(fixture(["GLIBC_2.17"], 183))
        self.assert_refusal(self.run_gate([rejected]), rejected, "architecture")

    def test_missing_image_has_bound_refusal(self):
        rejected = self.root / "missing"
        self.assert_refusal(self.run_gate([rejected]), rejected, "missing")

    def test_large_rejection_detail_keeps_stderr_bounded(self):
        rejected = self.root / "diskgraph"
        rejected.write_bytes(fixture(["GLIBC_2.17"], 62))
        maximum = "0." + "0." * 40000 + "0"
        stdout, stderr = io.StringIO(), io.StringIO()
        # 仅注入入口参数以跨越 Windows 命令行长度限制；检查器读取真实 ELF，未被替代。
        argv = [str(self.script), "--maximum-glibc", maximum,
                "--target", "x86_64-unknown-linux-gnu", str(rejected)]
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(stdout), \
                contextlib.redirect_stderr(stderr):
            with self.assertRaises(SystemExit) as exit_status:
                ABI.main()
        self.assertEqual(exit_status.exception.code, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertLessEqual(len(stderr.getvalue().encode("utf-8")), 65536)
        report = json.loads(stderr.getvalue())
        self.assertEqual(report["image"], str(rejected))
        self.assertTrue(report["diagnostic_truncated"])
        self.assertEqual(len(report["diagnostic_sha256"]), 64)

    def test_all_admitted_images_preserve_success_json_lines(self):
        paths = [self.root / name for name in ("diskgraph", "diskgraph-mcp")]
        for path in paths:
            path.write_bytes(fixture(["GLIBC_2.17"], 62))
        result = self.run_gate(paths)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        rows = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([row["image"] for row in rows], [p.name for p in paths])
        for row in rows:
            self.assertEqual(row["advertised_baseline"], "2.17")
            self.assertEqual(row["glibc_requirements"], ["2.17"])
            self.assertEqual(len(row["sha256"]), 64)


if __name__ == "__main__":
    unittest.main()
