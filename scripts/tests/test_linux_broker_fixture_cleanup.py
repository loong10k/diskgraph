"""隔离服务夹具失败时的文件保留策略；模拟systemctl，不是原生服务验收。"""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location('broker_fixture', SCRIPTS / 'qualify_linux_public_broker.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
import prepare_linux_supervisor_installation as INSTALLER


@unittest.skipUnless(os.name == 'posix', 'Unix service fixture cleanup only')
class FixtureCleanupTests(unittest.TestCase):
    def test_unconfirmed_unit_stop_keeps_unit_and_original_records(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            units = root / 'units'
            units.mkdir()
            prefix = root / 'installed'
            (prefix / 'state').mkdir(parents=True)
            for name in ['bootstrap.slot', 'uid_23111.slot']:
                (prefix / 'state' / name).write_bytes(b'DGSL01A\n')
            (prefix / 'diskgraph-supervisor.service').write_text('[Service]\n')
            retained = {'root': root, 'retain': False}
            actual_open = os.open
            def opened(path, flags, *args, **kwargs):
                return actual_open(units if str(path) == '/run/systemd/system' else path, flags, *args, **kwargs)
            def control(command, **_kwargs):
                if command[1] == 'start':
                    raise RuntimeError('original startup failed')
                if command[1] == 'stop':
                    raise subprocess.TimeoutExpired(command, 35)
                return subprocess.CompletedProcess(command, 0)
            with mock.patch.object(MODULE.os, 'open', side_effect=opened), \
                    mock.patch.object(INSTALLER, 'verify_root_directory'), \
                    mock.patch.object(MODULE.subprocess, 'run', side_effect=control):
                with self.assertRaises(subprocess.TimeoutExpired):
                    MODULE.systemd_installation(prefix, {'socket_path': str(root / 'unused')}, retained)
            self.assertTrue(retained['retain'])
            self.assertEqual(len(list(units.glob('dg-isolated-*.service'))), 1)
            for name in ['bootstrap.slot', 'uid_23111.slot']:
                self.assertEqual((prefix / 'state' / name).read_bytes(), b'DGSL01A\n')

    def test_retained_workspace_is_not_removed_on_exit(self):
        with tempfile.TemporaryDirectory() as parent:
            folder = Path(parent) / 'held'
            folder.mkdir()
            with mock.patch.object(MODULE.tempfile, 'mkdtemp', return_value=str(folder)), \
                    mock.patch.object(MODULE, 'print'):
                with MODULE.installed_workspace() as retained:
                    retained['retain'] = True
                    (folder / 'original.slot').write_bytes(b'DGSL01A\n')
            self.assertEqual((folder / 'original.slot').read_bytes(), b'DGSL01A\n')


if __name__ == '__main__':
    unittest.main()
