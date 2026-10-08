"""验收 Job 生命周期契约与真实 Windows 后代回收。"""
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

PATH = pathlib.Path(__file__).resolve().parents[1] / 'windows_acceptance_job.py'
SPEC = importlib.util.spec_from_file_location('acceptance_job_test', PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class OwnerContractTests(unittest.TestCase):
    def exercise(self, failure=None, retirement_failure=None, close_failure=None, owner_failure=None, alive=False):
        events = []
        job = mock.Mock()
        job.duplicate.side_effect = lambda: events.append('duplicate') or 42
        job.retire.side_effect = lambda deadline: (events.append('retire'),
            (_ for _ in ()).throw(retirement_failure) if retirement_failure else None)[-1]
        def close_owner():
            events.append('close')
            if owner_failure:
                raise owner_failure
        job.close.side_effect = close_owner
        def close_duplicate(handle):
            events.append('close_duplicate')
            if close_failure:
                raise close_failure
            return True
        job.api.CloseHandle.side_effect = close_duplicate

        process = mock.Mock()
        process.returncode = 0
        process.poll.return_value = None if alive else 0
        process.kill.side_effect = lambda: events.append("kill")
        process.stdout = process.stderr = None

        def communicate(timeout):
            events.append('communicate')
            self.assertGreater(timeout, 0)
            self.assertLessEqual(timeout, 300)
            if failure and events.count('communicate') == 1:
                raise failure
            if events.count('communicate') > 1:
                self.assertIn('retire', events)
                self.assertLessEqual(timeout, 5)
            return 'ok', ''

        process.communicate.side_effect = communicate

        def execute(command, **options):
            events.append('execute')
            self.assertEqual(options['startupinfo'].lpAttributeList, {'handle_list': [42]})
            self.assertTrue(options['close_fds'])
            self.assertNotIn('timeout', options)
            return process

        with mock.patch.object(MODULE, 'WindowsJob', return_value=job), \
             mock.patch.object(MODULE.subprocess, 'STARTUPINFO', types.SimpleNamespace, create=True), \
             mock.patch.object(MODULE.subprocess, 'Popen', side_effect=execute):
            try:
                result = MODULE.run_owned([sys.executable, 'fixture.py'], timeout=300)
            except BaseException as error:
                result = error
        expected = ['duplicate', 'execute', 'communicate', 'retire']
        if failure:
            if alive:
                expected.append("kill")
            expected.append('communicate')
        expected.extend(['close_duplicate', 'close'])
        self.assertEqual(events, expected)
        return result

    def test_success_retires_before_releasing_owner(self):
        self.assertEqual(self.exercise().returncode, 0)

    def test_timeout_preserves_original_exception_after_retirement(self):
        failure = subprocess.TimeoutExpired(['fixture'], 300)
        self.assertIs(self.exercise(failure), failure)

    def test_uncertain_retirement_preserves_original_and_reports_uncertainty(self):
        failure = subprocess.TimeoutExpired(['fixture'], 300)
        self.assertIs(self.exercise(failure, TimeoutError('still active')), failure)
        self.assertIn('unconfirmed', failure.__notes__[0])

    def test_duplicate_close_failure_does_not_mask_timeout_or_skip_owner(self):
        failure = subprocess.TimeoutExpired(['fixture'], 300)
        self.assertIs(self.exercise(failure, close_failure=OSError('duplicate close')), failure)
        self.assertIn('owner release unconfirmed', failure.__notes__[0])

    def test_owner_close_failure_preserves_timeout(self):
        failure = subprocess.TimeoutExpired(['fixture'], 300)
        self.assertIs(self.exercise(failure, owner_failure=OSError('owner close')), failure)
        self.assertIn('owner release unconfirmed', failure.__notes__[0])

    def test_retirement_failure_still_kills_original_unbound_wrapper(self):
        failure = subprocess.TimeoutExpired(['fixture'], 300)
        self.assertIs(self.exercise(failure, OSError('query failed'), alive=True), failure)

    def test_close_failure_after_success_cannot_return_success(self):
        error = OSError('owner close')
        self.assertIs(self.exercise(owner_failure=error), error)


@unittest.skipUnless(sys.platform == 'win32', 'real Windows Job required')
class NativeJobTests(unittest.TestCase):
    def test_real_descendant_retired_after_wrapper_timeout(self):
        with tempfile.TemporaryDirectory(prefix='diskgraph-job-native-') as directory:
            script = pathlib.Path(directory) / 'parent.py'
            marker = pathlib.Path(directory) / 'child_started'
            leaf = f'import pathlib,time; pathlib.Path({str(marker)!r}).write_text("ready"); time.sleep(60)'
            script.write_text('import subprocess,sys,time\n'
                              f'subprocess.Popen([sys.executable,"-c",{leaf!r}])\n'
                              'time.sleep(60)\n')
            with self.assertRaises(subprocess.TimeoutExpired) as caught:
                MODULE.run_owned([sys.executable, str(script), str(marker)],
                                 timeout=3, capture_output=True, text=True)
            self.assertFalse(getattr(caught.exception, '__notes__', []))
            self.assertTrue(marker.exists(), 'real descendant must start before timeout')


if __name__ == '__main__':
    unittest.main()
