"""原生wire夹具的生成/期限单元回归；不是broker运行验收。"""
import importlib.util
import json
from pathlib import Path
import sys
import types
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location('wire_client', Path(__file__).resolve().parents[1] / 'qualify_linux_broker_wire_client.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class WireFixtureTests(unittest.TestCase):
    def test_disconnect_uses_same_authenticated_read_only_query(self):
        namespace = types.SimpleNamespace(st_dev=4, st_ino=8)
        with mock.patch.object(MODULE.os, 'stat', return_value=namespace), \
                mock.patch.object(MODULE.time, 'monotonic_ns', return_value=123):
            self.assertEqual(MODULE.request('disconnect'), MODULE.request('doctor'))

    def test_negative_requests_keep_same_real_clock_shape(self):
        namespace = types.SimpleNamespace(st_dev=4, st_ino=8)
        with mock.patch.object(MODULE.os, 'stat', return_value=namespace), \
                mock.patch.object(MODULE.time, 'monotonic_ns', return_value=123):
            rows = [json.loads(MODULE.request(mode).split(b'\n', 1)[1]) for mode in
                    ('doctor', 'override', 'unknown_command', 'duplicate', 'bad_numeric', 'write', 'unknown_field')]
        for row in rows:
            self.assertEqual(row['deadline'], {'version': 1, 'domain': [1, 4, 8], 'expires_nanos': 25_000_000_123})
        self.assertEqual(rows[0]['query'], {'command': 'doctor', 'subcommand': None, 'options': [], 'switches': []})
        for row in rows[1:]:
            self.assertNotEqual(row, rows[0])
        self.assertEqual(rows[-1]['query'], rows[0]['query'])
        self.assertIn('environment', rows[-1])

    def test_partial_frames_deduct_original_deadline_each_read(self):
        connection = mock.Mock()
        connection.recv.side_effect = [b'a', b'b', b'c']
        with mock.patch.object(MODULE.time, 'monotonic', side_effect=[1, 2, 3]):
            self.assertEqual(MODULE.exact(connection, 3, 10), b'abc')
        self.assertEqual(connection.settimeout.call_args_list, [mock.call(9), mock.call(8), mock.call(7)])
        self.assertEqual(connection.recv.call_args_list, [mock.call(3), mock.call(2), mock.call(1)])

    def test_expired_or_truncated_frame_is_not_terminal_success(self):
        connection = mock.Mock()
        with mock.patch.object(MODULE.time, 'monotonic', return_value=10):
            with self.assertRaises(TimeoutError):
                MODULE.exact(connection, 4, 10)
        connection.recv.assert_not_called()
        connection.recv.return_value = b''
        with mock.patch.object(MODULE.time, 'monotonic', return_value=9):
            with self.assertRaisesRegex(AssertionError, 'truncated'):
                MODULE.exact(connection, 4, 10)


if __name__ == '__main__':
    unittest.main()
