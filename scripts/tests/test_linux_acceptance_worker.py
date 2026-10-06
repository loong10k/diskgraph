"""实际验收镜像不能随后续 Cargo 编译被替换。"""
import importlib.util
from pathlib import Path
import tempfile
import unittest

PATH = Path(__file__).resolve().parents[1] / 'configure_linux_acceptance_worker.py'
SPEC = importlib.util.spec_from_file_location('acceptance_worker', PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class WorkerSnapshot(unittest.TestCase):
    def artifact(self, path, kind='bin'):
        return [{'reason': 'compiler-artifact', 'target': {
            'name': 'diskgraph-scan-worker', 'kind': [kind]}, 'executable': str(path)}]

    def test_later_cargo_replacement_does_not_change_deployed_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            binary = root / 'cargo-worker'
            binary.write_bytes(b'original actual artifact')
            destination = root / 'deployed'
            receipt = MODULE.snapshot_worker(self.artifact(binary), destination)
            binary.write_bytes(b'later Cargo replacement')
            self.assertEqual(destination.read_bytes(), b'original actual artifact')
            self.assertEqual(receipt['worker_bytes'], destination.stat().st_size)
            self.assertEqual(receipt['worker_path'], str(destination))

    def test_protocol_example_cannot_supply_product_worker(self):
        with self.assertRaises(ValueError):
            MODULE.snapshot_worker(self.artifact(Path('/unused'), 'example'), Path('/unused-copy'))

    def test_existing_deployment_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source'
            source.write_bytes(b'worker')
            destination = root / 'deployed'
            destination.write_bytes(b'protected original')
            with self.assertRaises(FileExistsError):
                MODULE.snapshot_worker(self.artifact(source), destination)
            self.assertEqual(destination.read_bytes(), b'protected original')


if __name__ == '__main__':
    unittest.main()
