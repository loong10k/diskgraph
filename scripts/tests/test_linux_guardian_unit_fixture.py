"""unit启动失败的真实文件保留回归；模拟systemd命令，不宣称原生服务资格。"""
import contextlib
import importlib
import io
import json
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
    def test_mcp_mainline_accepts_actual_nested_wire_result(self):
        # 使用真实管道传递wire响应；模拟进程生命周期，不声明原生guardian验收。
        scan = {'scope_id': 'scope-original', 'revision_id': 'revision-original'}
        replies = [
            {'serverInfo': {'name': 'diskgraph'}},
            {'tools': [{'name': 'diskgraph_node'}]},
            {'isError': False, 'structuredContent': {
                'ok': True, 'scope_id': scan['scope_id'],
                'revision_id': scan['revision_id'],
                'data': {'node': {'files': 3}}}},
        ]
        for frontend, mismatch in ((frontend, mismatch) for frontend in ('mcp', 'cli')
                                   for mismatch in (None, 'scope_id', 'revision_id', 'node')):
            with self.subTest(frontend=frontend, mismatch=mismatch):
                responses = json.loads(json.dumps(replies))
                envelope = responses[2]['structuredContent']
                if mismatch == 'node':
                    envelope['node'] = envelope['data'].pop('node')
                elif mismatch:
                    envelope[mismatch] = 'different-original'
                output_read, output_write = os.pipe()
                error_read, error_write = os.pipe()
                with os.fdopen(output_read, 'rb') as stdout, os.fdopen(error_read, 'rb') as stderr:
                    os.write(output_write, b''.join(
                        (json.dumps({'jsonrpc': '2.0', 'id': number, 'result': reply}) + '\n').encode()
                        for number, reply in enumerate(responses, 1)))
                    os.close(output_write)
                    os.close(error_write)
                    child = mock.Mock(stdin=io.BytesIO(), stdout=stdout, stderr=stderr, returncode=0)
                    child.poll.return_value = None
                    child.communicate.return_value = (b'', b'')
                    with mock.patch.object(MODULE.subprocess, 'Popen', return_value=child), \
                            mock.patch.object(MODULE.time, 'sleep'), \
                            mock.patch.object(MODULE.os, 'pread', return_value=b'DGSL01C\n'), \
                            mock.patch.object(MODULE, 'original_dispose'):
                        if mismatch:
                            with self.assertRaisesRegex(AssertionError, 'MCP did not query actual customer revision'):
                                MODULE.qualify_mcp_mainline(Path('/fixture'), Path('/socket'), 1, scan, frontend)
                            child.communicate.assert_not_called()
                            continue
                        result = MODULE.qualify_mcp_mainline(Path('/fixture'), Path('/socket'), 1, scan, frontend)
                    self.assertTrue(result['actual_revision_query'])
                    self.assertTrue(result['original_slot_retired'])
                    child.communicate.assert_called_once_with(timeout=35)

    def test_mainline_scans_unicode_root_and_checks_raw_roots_as_refusals(self):
        # 夹具必须服从固定扫描器契约；模拟业务结果，不冒充原生执行。
        with tempfile.TemporaryDirectory() as folder:
            workspace = Path(folder)
            roots = {}
            revisions = []
            rejected = []
            def query(prefix, endpoint, slot, arguments, expected_success=True):
                command = arguments[0]
                if command == 'scope':
                    scope = 'scope-' + str(len(roots))
                    roots[scope] = Path(arguments[-1])
                    return {'data': {'scope_id': scope}}
                scope = arguments[arguments.index('--scope') + 1]
                if scope == 'unknown-scope':
                    self.assertFalse(expected_success)
                    return {'ok': False}
                raw = any(0xdc80 <= ord(char) <= 0xdcff for char in str(roots[scope]))
                if raw:
                    self.assertFalse(expected_success, '原始路径不能当成扫描器支持的成功发布')
                    rejected.append((scope, command))
                    return {'ok': False, 'error': {'code': 'unsupported' if command == 'index' else 'not_indexed'}}
                if command in ('index', 'sync'):
                    self.assertTrue(expected_success)
                    self.assertTrue(all(path.name.encode('utf-8') for path in roots[scope].iterdir()))
                    revisions.append('revision-' + str(len(revisions)))
                    return {'data': {'state': 'completed', 'revision_id': revisions[-1]}}
                if command == 'node':
                    return {'data': {'revision_id': revisions[-1], 'node': {'files': 3}}}
                if command == 'search':
                    return {'data': {'revision_id': revisions[-1], 'items': [{'name': 'İSTANBUL.txt'}]}}
                self.fail('unexpected command: ' + command)
            connection = mock.MagicMock()
            connection.__enter__.return_value.execute.return_value.fetchone.return_value = (2,)
            original_mkdir, original_write = Path.mkdir, Path.write_bytes
            def raw(path):
                return any(0xdc80 <= ord(char) <= 0xdcff for char in str(path))
            def mkdir(path, *args, **kwargs):
                if not raw(path):
                    return original_mkdir(path, *args, **kwargs)
            def write(path, data):
                return len(data) if raw(path) else original_write(path, data)
            with mock.patch.object(MODULE, 'customer_query', side_effect=query), \
                    mock.patch.object(MODULE.os, 'chown'), \
                    mock.patch.object(Path, 'mkdir', mkdir), \
                    mock.patch.object(Path, 'write_bytes', write), \
                    mock.patch.object(MODULE.sqlite3, 'connect', return_value=connection):
                result = MODULE.qualify_scan_mainline(workspace, workspace, workspace / 'socket', 1)
            self.assertTrue(result['actual_index_and_sync_completed'])
            self.assertEqual(len(revisions), 2)
            self.assertEqual(sum(command == 'index' for _, command in rejected), 2)
            self.assertEqual(sum(command == 'node' for _, command in rejected), 2)

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
