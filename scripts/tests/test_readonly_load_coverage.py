"""扫描完成标记不能替代负载夹具的实际节点覆盖。"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('load_coverage', ROOT / 'scripts/accept-readonly-load.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CoverageTests(unittest.TestCase):
    """模拟 CLI 协议，使用真实隔离 SQLite 证明部分结果不能计作完整负载。"""

    def exercise(self, count, duplicate=False, verify_closed=False):
        def invoke(cli, data, *arguments, **kwargs):
            if arguments[:2] == ('scope', 'add'):
                return {'data': {'scope_id': 'scope'}}
            if arguments[0] == 'index':
                data.mkdir()
                with sqlite3.connect(data / 'diskgraph.sqlite') as connection:
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
        test = self

        def record_reader(connection, *arguments):
            readers.append(connection)
            return real_coverage(connection, *arguments)

        class CheckedTemporaryDirectory(real_tempdir):
            def __exit__(self, *arguments):
                try:
                    test.assertTrue(readers, 'must observe actual read-only SQLite connection')
                    for connection in readers:
                        with test.assertRaises(sqlite3.ProgrammingError,
                                               msg='reader must close before fixture cleanup'):
                            connection.execute('SELECT 1')
                finally:
                    # 失败用例也关闭自己的真实句柄，避免红灯夹具残留。
                    for connection in readers:
                        connection.close()
                    super().__exit__(*arguments)

        output = io.StringIO()
        with patch.object(sys, 'argv', ['load', '--bin-dir', '.', '--files', '101', '--queries', '4']), \
             patch.object(MODULE, 'invoke', side_effect=invoke), \
             patch.object(MODULE, 'fixture_paths_complete', side_effect=record_reader), \
             patch.object(MODULE.tempfile, 'TemporaryDirectory',
                          CheckedTemporaryDirectory if verify_closed else real_tempdir), \
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

    def test_exact_files_and_root_coverage_is_accepted(self):
        result, _ = self.exercise(102)
        self.assertEqual(result, 0)


if __name__ == '__main__':
    unittest.main()
