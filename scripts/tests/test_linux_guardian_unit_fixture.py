"""unit启动失败的真实文件保留回归；模拟systemd命令，不宣称原生服务资格。"""
import contextlib
import importlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
MODULE = importlib.import_module('qualify_linux_guardian_unit')
INSTALLER = importlib.import_module('prepare_linux_supervisor_installation')


@unittest.skipUnless(os.name == 'posix', '该夹具验证POSIX原文件保留，不提供Windows服务验收')
class GuardianUnitFixtureTests(unittest.TestCase):
    def test_systemctl_failure_reports_original_error_without_retrying_start(self):
        failure = subprocess.CalledProcessError(1, ['systemctl', 'start', 'original.socket'],
                                                output='original stdout', stderr='original stderr')
        diagnostics = io.StringIO()
        with mock.patch.object(MODULE.subprocess, 'run', side_effect=[failure,
                subprocess.CompletedProcess([], 3, stdout='unit status', stderr=''),
                subprocess.CompletedProcess([], 0, stdout='unit journal', stderr='')]) as run, \
                contextlib.redirect_stderr(diagnostics):
            with self.assertRaises(subprocess.CalledProcessError) as caught:
                MODULE.systemctl('start', 'original.socket')
        self.assertIs(caught.exception, failure)
        self.assertIn('original stderr', diagnostics.getvalue())
        self.assertIn('unit status', diagnostics.getvalue())
        self.assertIn('unit journal', diagnostics.getvalue())
        self.assertEqual(sum(call.args[0][:2] == ['systemctl', 'start'] for call in run.call_args_list), 1)

    def test_activation_failure_keeps_exact_generated_units_and_original_slot(self):
        with tempfile.TemporaryDirectory() as folder:
            parent = Path(folder)
            units = parent / 'units'
            units.mkdir()
            workspace = parent / 'workspace'
            workspace.mkdir()
            prefix = workspace / 'installation'
            prefix.mkdir()
            (prefix / 'state').mkdir()
            (prefix / 'state/bootstrap.slot').write_bytes(b'DGSL01C\n')
            policy = INSTALLER.validate_identities(23111, 23111, [23112])
            images = {name: {'sha256': '1' * 64} for name in INSTALLER.NAMES}
            _, service, socket = INSTALLER.guardian_artifacts(Path('/var/lib/dg-unit-fixture'), policy, images)
            bodies = {'diskgraph-guardian.socket': socket.encode(),
                      'diskgraph-guardian@.service': service.encode()}
            for name, body in bodies.items():
                (prefix / name).write_bytes(body)
            failure = subprocess.CalledProcessError(1, ['systemctl', 'start'])
            original_open = os.open
            def open_original(path, *args, **kwargs):
                return original_open(units if path == '/run/systemd/system' else path, *args, **kwargs)
            def manager(*args):
                if args[0] == 'start':
                    raise failure
                return ''
            diagnostics = io.StringIO()
            with mock.patch.object(MODULE.sys, 'platform', 'linux'), \
                    mock.patch.object(MODULE.os, 'geteuid', return_value=0), \
                    mock.patch.dict(MODULE.os.environ, {'DG_ISOLATED_ROOT_ACCEPTANCE': '1'}), \
                    mock.patch.object(MODULE.tempfile, 'mkdtemp', return_value=str(workspace)), \
                    mock.patch.object(MODULE, 'install_candidate', return_value=prefix), \
                    mock.patch.object(MODULE, 'verify_root_directory'), \
                    mock.patch.object(MODULE.os, 'open', side_effect=open_original), \
                    mock.patch.object(MODULE.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)), \
                    mock.patch.object(MODULE.subprocess, 'Popen') as process, \
                    mock.patch.object(MODULE, 'systemctl', side_effect=manager), \
                    contextlib.redirect_stderr(diagnostics):
                with self.assertRaises(subprocess.CalledProcessError) as caught:
                    MODULE.qualify_unit(types.SimpleNamespace())
            self.assertIs(caught.exception, failure)
            process.assert_not_called()
            self.assertEqual(sorted(path.read_bytes() for path in units.iterdir()), sorted(bodies.values()))
            self.assertEqual((prefix / 'state/bootstrap.slot').read_bytes(), b'DGSL01C\n')
            self.assertIn('unconfirmed guardian unit retained', diagnostics.getvalue())
            self.assertTrue(prefix.exists())


if __name__ == '__main__':
    unittest.main()
