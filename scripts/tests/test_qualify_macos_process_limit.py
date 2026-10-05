"""监督器必须回收异常原进程，并保留主错及归档次错。"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('qualifier', Path(__file__).parents[1] / 'qualify_macos_process_limit.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FailedProcess:
    def __init__(self, primary, rescue_failure=False):
        self.returncode = None
        self.primary = primary
        self.rescue_failure = rescue_failure
        self.calls = []

    def communicate(self, timeout):
        self.calls.append('communicate')
        if self.calls.count('communicate') == 1:
            raise self.primary
        if self.rescue_failure:
            raise OSError(5, 'rescue')
        self.returncode = -9
        return b'raw-output', b'raw-error'

    def kill(self):
        self.calls.append('kill')
        if self.rescue_failure:
            raise OSError(5, 'kill')

    def wait(self, timeout):
        self.calls.append('wait')
        self.returncode = -9
        return self.returncode


class QualificationFailureTests(unittest.TestCase):
    def run_failure(self, primary, rescue_failure=False, archive_failure=False):
        process = FailedProcess(primary, rescue_failure)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source.c'
            binary = root / 'probe'
            source.write_bytes(b'source')
            binary.write_bytes(b'binary')
            original_write = Path.write_bytes

            def write_bytes(path, data):
                if archive_failure and path.name == 'stdout.bin':
                    raise OSError(28, 'artifact full')
                return original_write(path, data)

            with patch.object(MODULE.platform, 'platform', return_value='fixture-os'), \
                    patch.object(MODULE.subprocess, 'Popen', return_value=process), \
                    patch.object(Path, 'write_bytes', write_bytes), \
                    contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit):
                MODULE.qualify(binary, source, root / 'output', 'fixture')
            record = json.loads((root / 'output/result.json').read_text())
        self.assertEqual(record['primary_error']['kind'], type(primary).__name__)
        self.assertFalse(record['component_qualified'])
        self.assertTrue(record['original_process_waited'])
        self.assertEqual(process.calls, ['communicate', 'kill', 'communicate', 'wait'])
        return record

    def test_timeout_and_interrupt_and_io_error_reap_original(self):
        for error in [subprocess.TimeoutExpired('probe', 20), KeyboardInterrupt(), OSError(5, 'read')]:
            with self.subTest(kind=type(error).__name__):
                self.run_failure(error)

    def test_cleanup_errors_preserve_original_timeout(self):
        record = self.run_failure(subprocess.TimeoutExpired('probe', 20), rescue_failure=True)
        self.assertEqual(len(record['cleanup_errors']), 2)
        self.assertTrue(record['timed_out'])

    def test_archive_failure_preserves_primary(self):
        record = self.run_failure(KeyboardInterrupt(), archive_failure=True)
        self.assertEqual(record['artifact_errors'][0]['stage'], 'stdout')


if __name__ == '__main__':
    unittest.main()
