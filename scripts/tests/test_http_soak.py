"""持续请求夹具不能把错误结果或未满时长当作验收成功。"""
import importlib.util
import io
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'http_acceptance_soak', Path(__file__).resolve().parents[1] / 'accept-readonly-http.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class HttpSoakTests(unittest.TestCase):
    def test_response_byte_limit_rejects_oversize(self):
        self.assertEqual(len(MODULE.read_response(io.BytesIO(b'x' * (1024 * 1024)))),
                         1024 * 1024)
        with self.assertRaisesRegex(RuntimeError, 'exceeds 1 MiB'):
            MODULE.read_response(io.BytesIO(b'x' * (1024 * 1024 + 1)))

    def test_latency_storage_is_bounded_after_many_requests(self):
        now = [100.0]

        def read(*args, **kwargs):
            now[0] += 0.25
            return 200, {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [1]}}}}

        with patch.object(MODULE.time, 'monotonic', side_effect=lambda: now[0]), \
                patch.object(MODULE, 'request', side_effect=read):
            result = MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1250)
        self.assertEqual(result['requests'], 5000)
        self.assertEqual(result['latency_samples'], 4096)
        self.assertEqual(result['p95_ms'], 250)

    def test_runs_until_original_deadline_and_limits_each_request(self):
        now = [100.0]
        calls = []

        def read(*args, **kwargs):
            calls.append(kwargs['timeout'])
            now[0] += 0.25
            return 200, {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [{'id': 1}]}}}}

        with patch.object(MODULE.time, 'monotonic', side_effect=lambda: now[0]), \
                patch.object(MODULE, 'request', side_effect=read):
            result = MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        self.assertEqual(calls, [1.0, 0.75, 0.5, 0.25])
        self.assertEqual(result['requests'], 4)
        self.assertEqual(result['elapsed_seconds'], 1.0)
        self.assertFalse(result['native_long_run_qualified'])

    def test_wrong_scope_and_permission_error_are_not_success(self):
        answers = [
            (200, {'result': {'structuredContent': {
                'scope_id': 'scope-b', 'data': {'items': [1]}}}}),
            (200, {'error': {'data': {'business_code': 'permission_denied'}}}),
            (500, {}),
        ]
        for answer in answers:
            with self.subTest(answer=answer), \
                    patch.object(MODULE, 'request', return_value=answer):
                with self.assertRaisesRegex(RuntimeError, 'soak read'):
                    MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)

    def test_request_error_is_preserved(self):
        original = TimeoutError('original request timeout')
        with patch.object(MODULE, 'request', side_effect=original):
            with self.assertRaises(TimeoutError) as raised:
                MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        self.assertIs(raised.exception, original)

    def test_rejects_nonfinite_or_nonpositive_duration_before_request(self):
        for seconds in (0, -1, float('nan'), float('inf'), 86401):
            with self.subTest(seconds=seconds), patch.object(MODULE, 'request') as read:
                with self.assertRaises(ValueError):
                    MODULE.soak_reads(1234, {}, 'key', 'scope-a', seconds)
                read.assert_not_called()
