"""包验收使用独立构建预期，并执行解包后的真实 worker。"""
import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location('package_acceptance', SCRIPTS / 'accept-readonly-package.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PackageWorkerDeployment(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.bin = Path(self.temp.name)
        self.worker = self.bin / 'diskgraph-scan-worker'
        self.body = b'current controlled build worker'
        self.worker.write_bytes(self.body)
        self.environment = {
            'DISKGRAPH_SCAN_WORKER_PATH': '/controlled-build/snapshot',
            'DISKGRAPH_SCAN_WORKER_SHA256': hashlib.sha256(self.body).hexdigest(),
            'DISKGRAPH_SCAN_WORKER_BYTES': str(len(self.body)),
            'KEEP_EXISTING': 'yes',
        }

    def test_executes_packaged_worker_with_independent_expectations(self):
        result = MODULE.packaged_worker_environment(self.bin, self.environment)
        self.assertEqual(result['DISKGRAPH_SCAN_WORKER_PATH'], str(self.worker))
        self.assertEqual(result['DISKGRAPH_SCAN_WORKER_SHA256'], self.environment['DISKGRAPH_SCAN_WORKER_SHA256'])
        self.assertEqual(result['KEEP_EXISTING'], 'yes')
        self.assertEqual(self.environment['DISKGRAPH_SCAN_WORKER_PATH'], '/controlled-build/snapshot')

    def test_child_acceptance_receives_packaged_worker_environment(self):
        deployment = MODULE.packaged_worker_environment(self.bin, self.environment)
        completed = mock.Mock(returncode=0, stdout='{"passed": 1, "total": 1}', stderr='')
        with mock.patch.object(MODULE.subprocess, 'run', return_value=completed) as run:
            self.assertEqual(MODULE.accepted('accept-readonly-stdio.py', self.bin,
                                             deployment=deployment), 1)
        environment = run.call_args.kwargs['env']
        self.assertEqual(environment['DISKGRAPH_SCAN_WORKER_PATH'], str(self.worker))
        self.assertEqual(environment['DISKGRAPH_ACCEPT_BIN_DIR'], str(self.bin))
        self.assertNotIn('DISKGRAPH_ACCEPT_BIN_DIR', deployment)

    def test_missing_independent_expected_values_rejected(self):
        for key in ('PATH', 'SHA256', 'BYTES'):
            environment = self.environment.copy()
            del environment['DISKGRAPH_SCAN_WORKER_' + key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                MODULE.packaged_worker_environment(self.bin, environment)

    def test_package_cannot_supply_its_own_expected_digest(self):
        self.worker.write_bytes(b'replaced image')
        (self.bin / 'scan-worker-manifest.json').write_text('{}')
        with self.assertRaises(ValueError):
            MODULE.packaged_worker_environment(self.bin, self.environment)

    def test_length_mismatch_rejected(self):
        self.environment['DISKGRAPH_SCAN_WORKER_BYTES'] = str(len(self.body) + 1)
        with self.assertRaises(ValueError):
            MODULE.packaged_worker_environment(self.bin, self.environment)

    def test_malformed_deployment_rejected(self):
        for key, value in [('SHA256', 'g' * 64), ('BYTES', '-1'), ('BYTES', 'unknown'), ('PATH', 'relative')]:
            environment = self.environment.copy()
            environment['DISKGRAPH_SCAN_WORKER_' + key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                MODULE.packaged_worker_environment(self.bin, environment)

    def test_linked_packaged_worker_rejected(self):
        original = self.bin / 'original'
        self.worker.rename(original)
        self.worker.symlink_to(original)
        with self.assertRaises(ValueError):
            MODULE.packaged_worker_environment(self.bin, self.environment)


if __name__ == '__main__':
    unittest.main()
