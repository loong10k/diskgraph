"""扫描完成标记不能替代负载夹具的实际节点覆盖。"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import threading
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('load_coverage', ROOT / 'scripts/accept-readonly-load.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FixtureCreationTests(unittest.TestCase):
    """并行准备仍创建完整真实文件，写入错误不能变成成功。"""
    def test_exact_names_and_contents_with_non_multiple_file_count(self):
        with MODULE.tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            MODULE.create_fixture(root, 103)
            self.assertEqual({p.name for p in root.iterdir()},
                             {f'file-{i:06}.bin' for i in range(103)})
            self.assertTrue(all(p.read_bytes() == b'x' * 32 for p in root.iterdir()))

    def test_diagnostic_worker_counts_preserve_identical_real_files(self):
        for workers in (1, 2, 4):
            with self.subTest(workers=workers), MODULE.tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                MODULE.create_fixture(root, 103, workers=workers)
                self.assertEqual({p.name for p in root.iterdir()},
                                 {f'file-{i:06}.bin' for i in range(103)})
                self.assertTrue(all(p.read_bytes() == b'x' * 32 for p in root.iterdir()))

    def test_diagnostic_worker_count_refuses_outside_bounded_range(self):
        for workers in (0, 5):
            with self.subTest(workers=workers), MODULE.tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                with self.assertRaises(ValueError):
                    MODULE.create_fixture(root, 103, workers=workers)
                self.assertEqual(list(root.iterdir()), [])

    def test_preparation_uses_at_most_four_workers_and_overlaps_io(self):
        barrier = threading.Barrier(4, timeout=2)
        lock = threading.Lock()
        threads = set()
        real_write = Path.write_bytes

        def write(path, content):
            identity = threading.get_ident()
            with lock:
                first = identity not in threads
                threads.add(identity)
            if first:
                barrier.wait()
            return real_write(path, content)

        with MODULE.tempfile.TemporaryDirectory() as temporary:
            with patch.object(Path, 'write_bytes', write):
                MODULE.create_fixture(Path(temporary), 103)
            self.assertEqual(len(list(Path(temporary).iterdir())), 103)
        self.assertEqual(len(threads), 4)

    def test_write_failure_propagates(self):
        with MODULE.tempfile.TemporaryDirectory() as temporary:
            error = OSError('fixture storage unavailable')
            with patch.object(Path, 'write_bytes', side_effect=error):
                with self.assertRaises(OSError) as caught:
                    MODULE.create_fixture(Path(temporary), 103)
            self.assertIs(caught.exception, error)


class CoverageTests(unittest.TestCase):
    """模拟 CLI 协议，使用真实隔离 SQLite 证明部分结果不能计作完整负载。"""

    def exercise(self, count, duplicate=False, verify_closed=False, verify_all_closed=False):
        def invoke(cli, data, *arguments, **kwargs):
            if arguments[:2] == ('scope', 'add'):
                return {'data': {'scope_id': 'scope'}}
            if arguments[0] == 'index':
                data.mkdir()
                with contextlib.closing(sqlite3.connect(data / 'diskgraph.sqlite')) as connection, connection:
                    connection.execute('CREATE TABLE graph_revisions(revision_id TEXT, snapshot_id TEXT)')
                    connection.execute('CREATE TABLE nodes(snapshot_id TEXT, id INTEGER, parent_id INTEGER, name TEXT)')
                    connection.execute("INSERT INTO graph_revisions VALUES('revision','snapshot')")
                    rows = [('snapshot', 0, None, 'project')]
                    rows.extend(('snapshot', index + 1, 0, f'file-{index:06}.bin') for index in range(count - 1))
                    if duplicate:
                        rows[-1] = ('snapshot', count - 1, 0, 'file-000000.bin')
                    connection.executemany('INSERT INTO nodes VALUES(?,?,?,?)', rows)
                return {'data': {'state': 'completed', 'revision_id': 'revision'}}
            if arguments[0] == '--max-nodes-per-scan':
                return {'error': {'code': 'budget_exceeded'}}
            if arguments[0] == 'tree':
                return {'error': {'code': 'not_indexed'}}
            return {'ok': True, 'data': {'revision_id': 'revision', 'items': [1]}}

        real_tempdir = MODULE.tempfile.TemporaryDirectory
        real_coverage = MODULE.fixture_paths_complete
        readers = []
        connections = []
        real_connect = sqlite3.connect
        test = self

        def record_connection(*arguments, **keywords):
            connection = real_connect(*arguments, **keywords)
            connections.append(connection)
            return connection

        def record_reader(connection, *arguments):
            readers.append(connection)
            return real_coverage(connection, *arguments)

        class CheckedTemporaryDirectory(real_tempdir):
            def __exit__(self, *arguments):
                try:
                    test.assertTrue(readers, 'must observe actual read-only SQLite connection')
                    observed = connections if verify_all_closed else readers
                    for connection in observed:
                        with test.assertRaises(sqlite3.ProgrammingError,
                                               msg='every observed connection must close before fixture cleanup'):
                            connection.execute('SELECT 1')
                finally:
                    # 失败用例也关闭自己的真实句柄，避免红灯夹具残留。
                    for connection in connections:
                        connection.close()
                    super().__exit__(*arguments)

        output = io.StringIO()
        with patch.object(sys, 'argv', ['load', '--bin-dir', '.', '--files', '101', '--queries', '4']), \
             patch.object(MODULE, 'invoke', side_effect=invoke), \
             patch.object(MODULE, 'fixture_paths_complete', side_effect=record_reader), \
             patch.object(MODULE.sqlite3, 'connect', side_effect=record_connection), \
             patch.object(MODULE.tempfile, 'TemporaryDirectory',
                          CheckedTemporaryDirectory if verify_closed or verify_all_closed else real_tempdir), \
             patch.object(MODULE.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'diskgraph test\n')), \
             contextlib.redirect_stdout(output):
            result = MODULE.main()
        return result, json.loads(output.getvalue())

    def test_completed_but_underpopulated_snapshot_is_refused(self):
        result, _ = self.exercise(2)
        self.assertEqual(result, 1, 'two nodes must not qualify as a 101-file load')

    def test_equal_count_with_missing_and_duplicate_paths_is_refused(self):
        result, _ = self.exercise(102, duplicate=True)
        self.assertEqual(result, 1, 'equal counts must not hide a missing fixture file')

    def test_readonly_connection_closes_before_fixture_cleanup(self):
        result, _ = self.exercise(102, verify_closed=True)
        self.assertEqual(result, 0)

    def test_writer_and_reader_close_before_fixture_cleanup(self):
        result, _ = self.exercise(102, verify_all_closed=True)
        self.assertEqual(result, 0)

    def test_platform_workers_are_forwarded_and_reported(self):
        for platform, expected in [('win32', 2), ('linux', 4), ('darwin', 4)]:
            with self.subTest(platform=platform), \
                 patch.object(MODULE.sys, 'platform', platform), \
                 patch.object(MODULE, 'create_fixture', wraps=MODULE.create_fixture) as create:
                result, report = self.exercise(102)
                self.assertEqual(result, 0)
                self.assertEqual(create.call_args.kwargs, {'workers': expected})
                self.assertEqual(report['fixture_workers'], expected)

    def test_exact_files_and_root_coverage_is_accepted(self):
        result, _ = self.exercise(102)
        self.assertEqual(result, 0)


if __name__ == '__main__':
    unittest.main()
