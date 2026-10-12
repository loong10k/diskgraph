"""guardian安装模式的可观察布局回归；不代替原生systemd运行验收。"""
import os
from pathlib import Path
import tempfile
import unittest

import test_linux_supervisor_installation as fixtures

MODULE = fixtures.MODULE


class GuardianInstallationTests(unittest.TestCase):
    def test_guardian_images_are_executable_but_unreadable_to_customer(self):
        fixture = fixtures.ArchiveTests()
        with tempfile.TemporaryDirectory() as folder:
            fd = os.open(folder, os.O_RDONLY)
            try:
                images, _ = MODULE.install_images(fixture.package(with_broker=True), fd,
                    fixture.target, with_broker=True, with_guardian=True)
                for name in ('diskgraph', 'diskgraph-mcp', 'diskgraph-scan-worker'):
                    self.assertEqual(images[name]['mode'], 0o511)
                self.assertEqual(images['diskgraph-supervisor']['mode'], 0o555)
            finally:
                os.close(fd)

    def test_guardian_socket_uses_original_credentials_and_bounded_admission(self):
        policy = MODULE.validate_identities(23111, 23111, [23112])
        images = {name: {'sha256': '1' * 64} for name in MODULE.NAMES}
        config, service, socket = MODULE.guardian_artifacts(Path('/var/lib/dg-test'), policy, images)
        self.assertEqual(config['frontend_uid'], 23112)
        self.assertIn('--connection', service)
        self.assertIn('StandardInput=socket', service.splitlines())
        for value in ('Accept=yes', 'PassCredentials=yes', 'MaxConnections=4',
                      'MaxConnectionsPerSource=4', 'SocketMode=0600', 'SocketUser=23112'):
            self.assertIn(value, socket.splitlines())
        self.assertIn('KillMode=mixed', service.splitlines())
        self.assertIn('SendSIGKILL=no', service.splitlines())
        self.assertIn('TimeoutStopSec=infinity', service.splitlines())
        self.assertNotIn('ProtectHome=yes', service)
        self.assertNotIn('Restart=always', service)

    def test_conflicting_modes_and_missing_customer_gid_refuse_before_files(self):
        for options in ({'with_broker': True, 'with_guardian': True, 'frontend_gid': 23112},
                        {'with_guardian': True}, {'with_guardian': True, 'frontend_gid': 0}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                MODULE.prepare(Path('/missing-source'), '1' * 64, Path('/var/lib/missing-install'),
                               23111, 23111, [23112], **options)

    def test_listen_stream_uses_literal_path_without_exec_argument_quotes(self):
        policy = MODULE.validate_identities(23111, 23111, [23112])
        images = {name: {'sha256': '1' * 64} for name in MODULE.NAMES}
        prefix = '/var/lib/dg %n "quoted"'
        _, _, socket = MODULE.guardian_artifacts(Path(prefix), policy, images)
        actual = next(line.split('=', 1)[1] for line in socket.splitlines() if line.startswith('ListenStream='))
        self.assertEqual(actual, prefix.replace('%', '%%') + '/front.sock')


if __name__ == '__main__':
    unittest.main()
