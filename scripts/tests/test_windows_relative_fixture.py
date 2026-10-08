"""原父句柄夹具的名称、ABI 与原生拒绝行为；不代替完整负载验收。"""
import ctypes
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('relative_fixture',
    Path(__file__).resolve().parents[1] / 'windows_relative_fixture.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FixtureContractTests(unittest.TestCase):
    def test_names_match_existing_workload_and_reject_paths_or_bounds(self):
        self.assertEqual(MODULE.fixture_name(0), 'file-000000.bin')
        self.assertEqual(MODULE.fixture_name(199999), 'file-199999.bin')
        for value in (-1, 200000, True, False, 'file-000000.bin', 1.0, None):
            with self.subTest(value=value), self.assertRaises(ValueError):
                MODULE.fixture_name(value)

    def test_count_rejects_empty_overflow_and_non_integer_workloads(self):
        for value in (0, -1, 200001, True, False, '1', 1.0, None):
            with self.subTest(value=value), self.assertRaises(ValueError):
                MODULE.WindowsRelativeFixture._validate_count(value)
        MODULE.WindowsRelativeFixture._validate_count(1)
        MODULE.WindowsRelativeFixture._validate_count(200000)

    def test_abi_retains_native_boolean_and_pointer_alignment(self):
        self.assertEqual(MODULE.StandardInformation.links.offset, 16)
        self.assertEqual(MODULE.StandardInformation.delete_pending.offset, 20)
        self.assertEqual(MODULE.StandardInformation.directory.offset, 21)
        self.assertEqual(ctypes.sizeof(MODULE.StandardInformation), 24)
        self.assertEqual(ctypes.sizeof(MODULE.AttributeTagInformation), 8)
        pointer_bytes = ctypes.sizeof(ctypes.c_void_p)
        self.assertEqual(ctypes.sizeof(MODULE.IoStatusBlock), pointer_bytes * 2)
        self.assertEqual(ctypes.sizeof(MODULE.ObjectAttributes), 48 if pointer_bytes == 8 else 24)


@unittest.skipUnless(os.name == 'nt', 'requires actual Windows filesystem handles')
class NativeFixtureTests(unittest.TestCase):
    def test_exact_contents_names_links_and_actual_retirement(self):
        with tempfile.TemporaryDirectory(prefix='diskgraph-relative-contract-') as temporary:
            root = Path(temporary)
            with MODULE.WindowsRelativeFixture(root) as fixture:
                fixture.create(17)
                self.assertEqual({p.name for p in root.iterdir()},
                                 {MODULE.fixture_name(i) for i in range(17)})
                for path in root.iterdir():
                    self.assertEqual(path.read_bytes(), b'x' * 32)
                    self.assertEqual(path.stat().st_nlink, 1)
                fixture.remove(17)
                self.assertEqual(list(root.iterdir()), [])
            with self.assertRaises(OSError):
                fixture.create(1)

    def test_duplicate_creation_never_overwrites_existing_bytes(self):
        with tempfile.TemporaryDirectory(prefix='diskgraph-relative-duplicate-') as temporary:
            root = Path(temporary)
            original = root / MODULE.fixture_name(0)
            original.write_bytes(b'original bytes')
            with MODULE.WindowsRelativeFixture(root) as fixture:
                with self.assertRaises(OSError):
                    fixture.create(1)
                self.assertEqual(original.read_bytes(), b'original bytes')

    def test_multiple_links_are_refused_before_deletion(self):
        with tempfile.TemporaryDirectory(prefix='diskgraph-relative-links-') as temporary:
            root = Path(temporary)
            with MODULE.WindowsRelativeFixture(root) as fixture:
                fixture.create(1)
                original = root / MODULE.fixture_name(0)
                alias = root / 'foreign-alias.bin'
                os.link(original, alias)
                with self.assertRaises(OSError):
                    fixture.remove(1)
                self.assertEqual(original.read_bytes(), b'x' * 32)
                self.assertEqual(alias.read_bytes(), b'x' * 32)
                alias.unlink()
                fixture.remove(1)
                self.assertEqual(list(root.iterdir()), [])

    def test_directory_with_fixture_name_is_not_adopted_or_removed(self):
        with tempfile.TemporaryDirectory(prefix='diskgraph-relative-directory-') as temporary:
            root = Path(temporary)
            foreign = root / MODULE.fixture_name(0)
            foreign.mkdir()
            sentinel = foreign / 'keep.bin'
            sentinel.write_bytes(b'keep')
            with MODULE.WindowsRelativeFixture(root) as fixture:
                with self.assertRaises(OSError):
                    fixture.remove(1)
                self.assertEqual(sentinel.read_bytes(), b'keep')


if __name__ == '__main__':
    unittest.main()
