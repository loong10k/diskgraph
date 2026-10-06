"""密封镜像门禁必须绑定当前源码，不覆盖为历史候选。"""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'qualify-linux-scan-image.py'
SPEC = importlib.util.spec_from_file_location('sealed_image_gate', SCRIPT)
GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GATE)


class CurrentSources(unittest.TestCase):
    def prepare(self, root):
        native = root / 'crates/diskgraph-engine/src/native_child'
        native.mkdir(parents=True)
        names = ('linux_scan_image', 'linux_scan_image_tests',
                 'linux_scan_image_fixture', 'linux_scan_image_error')
        (native / 'mod.rs').write_text(''.join(f'mod {name};\n' for name in names))
        for name in names:
            (native / f'{name}.rs').write_text(f'// current {name}\n')
        return native

    def test_current_bytes_are_recorded_without_mutation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            native = self.prepare(root)
            before = {p.name: p.read_bytes() for p in native.iterdir()}
            receipt = GATE.current_sources(root)
            self.assertEqual(len(receipt), 5)
            self.assertEqual(before, {p.name: p.read_bytes() for p in native.iterdir()})
            self.assertTrue(all(item['path'].startswith('crates/diskgraph-engine/src/')
                                for item in receipt))

    def test_duplicate_module_is_rejected_before_cargo(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            native = self.prepare(root)
            with (native / 'mod.rs').open('a') as out:
                out.write('mod linux_scan_image;\n')
            with self.assertRaisesRegex(RuntimeError, 'exactly once'):
                GATE.current_sources(root)

    def test_missing_current_source_is_not_replaced(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            native = self.prepare(root)
            (native / 'linux_scan_image.rs').unlink()
            with self.assertRaises(FileNotFoundError):
                GATE.current_sources(root)


if __name__ == '__main__':
    unittest.main()
