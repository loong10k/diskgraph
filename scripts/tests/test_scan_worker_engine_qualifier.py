"""资格编排的原命令失败不能被日志关闭次错覆盖。"""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).parents[1]
SPEC = importlib.util.spec_from_file_location('engine_qualifier', SCRIPTS / 'qualify-linux-scan-worker-engine.py')
MODULE = importlib.util.module_from_spec(SPEC)
with patch.dict(sys.modules):
    assembly_spec = importlib.util.spec_from_file_location('assemble_scan_worker_host', SCRIPTS / 'assemble_scan_worker_host.py')
    assembly = importlib.util.module_from_spec(assembly_spec)
    assembly_spec.loader.exec_module(assembly)
    sys.modules['assemble_scan_worker_host'] = assembly
    SPEC.loader.exec_module(MODULE)


class EngineQualifierErrorContracts(unittest.TestCase):
    def test_failed_original_command_survives_close_error(self):
        primary = subprocess.CalledProcessError(101, ['cargo', 'test'])
        receipt = {}
        class FailedLog:
            def __enter__(self):
                return self
            def __exit__(self, kind, value, traceback):
                self.close()
            def close(self):
                raise OSError(5, 'actual close error fixture')
        with patch.object(Path, 'open', return_value=FailedLog()), \
                patch.object(MODULE.subprocess, 'run', side_effect=primary), \
                self.assertRaises(subprocess.CalledProcessError) as raised:
            MODULE.run_step(Path('/unused'), Path('/unused'), {}, receipt, 'recovery', ['cargo', 'test'])
        self.assertIs(raised.exception, primary)
        self.assertEqual(receipt['secondary_errors'], [{'stage': 'recovery-log-close', 'kind': 'OSError', 'errno': 5}])

    def test_close_failure_after_success_still_refuses_qualification(self):
        class FailedLog:
            def __enter__(self):
                return self
            def __exit__(self, kind, value, traceback):
                self.close()
            def close(self):
                raise OSError(5, 'close error')
        with patch.object(Path, 'open', return_value=FailedLog()), \
                patch.object(MODULE.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)), \
                self.assertRaises(OSError):
            MODULE.run_step(Path('/unused'), Path('/unused'), {}, {}, 'build', [])


if __name__ == '__main__':
    unittest.main()
