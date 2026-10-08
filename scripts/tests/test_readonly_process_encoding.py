"""真实子进程的 UTF-8 JSON 与错误信息不能依赖宿主默认代码页。"""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "readonly_encoding", Path(__file__).resolve().parents[1] / "accept-readonly-stdio.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReadonlyProcessEncodingTests(unittest.TestCase):
    def test_real_utf8_json_survives_gbk_host_default(self):
        payload = '{"name":"中文😀"}\n'.encode("utf-8")
        with patch.object(subprocess, "_text_encoding", return_value="gbk"):
            result = MODULE.run(sys.executable, "-c", f"import os; os.write(1, {payload!r})")
        self.assertEqual(result, [{"name": "中文😀"}])

    def test_real_utf8_error_preserves_exit_code_and_diagnostic(self):
        diagnostic = "权限失败：😀"
        payload = diagnostic.encode("utf-8")
        with patch.object(subprocess, "_text_encoding", return_value="gbk"):
            with self.assertRaises(RuntimeError) as raised:
                MODULE.run(sys.executable, "-c",
                           f"import os,sys; os.write(2, {payload!r}); sys.exit(6)")
        self.assertIn("exited 6", str(raised.exception))
        self.assertIn(diagnostic, str(raised.exception))


if __name__ == "__main__":
    unittest.main(verbosity=2)
