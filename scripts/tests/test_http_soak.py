"""持续请求夹具不能把错误结果或未满时长当作验收成功。"""
import importlib.util
import io
import contextlib
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'http_acceptance_soak', Path(__file__).resolve().parents[1] / 'accept-readonly-http.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class HttpSoakTests(unittest.TestCase):
    def test_fast_reads_are_paced_without_bursts_or_extending_deadline(self):
        now = [100.0]
        starts = []

        def read(*args, **kwargs):
            starts.append(now[0])
            return 200, {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [1]}}}}

        def sleep(seconds):
            self.assertGreater(seconds, 0)
            now[0] += seconds

        # 有限时钟观测也防止无节流实现陷入无限循环。
        observations = [0]
        def clock():
            observations[0] += 1
            if observations[0] > 1000:
                self.fail('fast reads spun without advancing the original deadline')
            return now[0]

        with patch.object(MODULE.time, 'monotonic', side_effect=clock), \
                patch.object(MODULE.time, 'sleep', side_effect=sleep), \
                patch.object(MODULE, 'request', side_effect=read):
            result = MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        self.assertTrue(39 <= len(starts) <= 41)
        self.assertTrue(all(b - a >= 0.025 - 1e-10 for a, b in zip(starts, starts[1:])))
        self.assertEqual(result['elapsed_seconds'], 1.0)
        self.assertEqual(result['offered_max_requests_per_second'], 40)
        self.assertEqual(result['dispatch_reserve_ms'], 25.0)

    def test_final_partial_dispatch_interval_is_observed_without_new_request(self):
        now = [100.0]
        requests = []
        sleeps = []

        def read(*args, **kwargs):
            requests.append(now[0])
            # 复现原生报告：已成功请求后仅剩3.3ms，不足一次25ms发起间隔。
            now[0] = 100.9967
            return 200, {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [1]}}}}

        def sleep(seconds):
            sleeps.append(seconds)
            now[0] += seconds

        def clock():
            if len(requests) > 1:
                self.fail('issued a request without one remaining dispatch interval')
            return now[0]

        with patch.object(MODULE.time, 'monotonic', side_effect=clock), \
                patch.object(MODULE.time, 'sleep', side_effect=sleep), \
                patch.object(MODULE, 'request', side_effect=read):
            result = MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        self.assertEqual(len(requests), 1)
        self.assertAlmostEqual(sum(sleeps), 0.0033)
        self.assertEqual(result['elapsed_seconds'], 1.0)
        self.assertAlmostEqual(result['dispatch_reserve_ms'], 996.7)

    def test_token_cost_must_leave_a_dispatch_interval_before_network(self):
        now = [100.0]
        minted = [0]

        def mint(key):
            minted[0] += 1
            if minted[0] == 2:
                now[0] = 100.9967
            return 'same-principal-token'

        def read(*args, **kwargs):
            self.assertEqual(minted[0], 1, 'late token preparation must prevent dispatch')
            now[0] += 0.25
            return 200, {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [1]}}}}

        with patch.object(MODULE.time, 'monotonic', side_effect=lambda: now[0]), \
                patch.object(MODULE.time, 'sleep', side_effect=lambda t: now.__setitem__(0, now[0] + t)), \
                patch.object(MODULE, 'token', side_effect=mint), \
                patch.object(MODULE, 'request', side_effect=read) as request:
            result = MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        request.assert_called_once()
        self.assertEqual(result['elapsed_seconds'], 1.0)

    def test_observed_request_cost_is_reserved_on_slower_platforms(self):
        now = [100.0]
        minted = [0]

        def mint(key):
            minted[0] += 1
            if minted[0] == 2:
                now[0] = 100.972
            return 'same-principal-token'

        def read(*args, **kwargs):
            self.assertEqual(minted[0], 1, '28ms cannot admit the observed 40ms request cost')
            now[0] += 0.040
            return 200, {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [1]}}}}

        with patch.object(MODULE.time, 'monotonic', side_effect=lambda: now[0]), \
                patch.object(MODULE.time, 'sleep', side_effect=lambda t: now.__setitem__(0, now[0] + t)), \
                patch.object(MODULE, 'token', side_effect=mint), \
                patch.object(MODULE, 'request', side_effect=read) as request:
            result = MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        request.assert_called_once()
        self.assertEqual(result['elapsed_seconds'], 1.0)
        self.assertAlmostEqual(result['dispatch_reserve_ms'], 40.0)

    def test_rate_limit_rejection_is_failure_and_is_not_retried(self):
        with patch.object(MODULE, 'request', return_value=(429, {
                'error': 'rate_limited', 'retry_after_ms': 20})) as read:
            with self.assertRaisesRegex(RuntimeError, 'http_status.*429'):
                MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        read.assert_called_once()

    def test_non_list_items_and_result_with_rpc_error_cannot_pass(self):
        answers = [
            {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': 'malformed'}}}},
            {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': {'foreign': 1}}}}},
            {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [1]}}},
             'error': {'data': {'business_code': 'permission_denied'}}},
        ]
        for answer in answers:
            with self.subTest(answer=answer), \
                    patch.object(MODULE, 'request', return_value=(200, answer)) as read:
                with self.assertRaisesRegex(RuntimeError, 'soak read'):
                    MODULE.soak_reads(1234, {}, 'key', 'scope-a', 0.03)
                read.assert_called_once()

    def test_budget_failure_reports_only_safe_classification_and_never_retries(self):
        answer = {'error': {'message': 'private path /private/example',
                            'data': {'business_code': 'budget_exceeded',
                                     'principal': 'private-subject'}}}
        with patch.object(MODULE, 'request', return_value=(200, answer)) as read:
            with self.assertRaisesRegex(RuntimeError, 'business_code.*budget_exceeded') as raised:
                MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        read.assert_called_once()
        self.assertNotIn('/private/example', str(raised.exception))
        self.assertNotIn('private-subject', str(raised.exception))

    def test_malformed_response_and_unknown_error_text_cannot_enter_diagnostics(self):
        answers = [[], {'error': {'data': {'business_code': '/private/example\nsecret'}}},
                   {'result': {'structuredContent': 'private-content'}}]
        for answer in answers:
            with self.subTest(answer=answer), patch.object(MODULE, 'request', return_value=(200, answer)):
                with self.assertRaisesRegex(RuntimeError, 'soak read') as raised:
                    MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
                self.assertNotIn('/private/example', str(raised.exception))
                self.assertNotIn('private-content', str(raised.exception))

    def test_binary_evidence_detects_replaced_content_without_loading_whole_image(self):
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / 'mcp'
            image.write_bytes(b'first image')
            first = MODULE.binary_evidence(image)
            self.assertEqual(first['bytes'], 11)
            self.assertEqual(first['sha256'], MODULE.hashlib.sha256(b'first image').hexdigest())
            image.write_bytes(b'second image')
            self.assertNotEqual(first, MODULE.binary_evidence(image))

    def test_binary_evidence_rejects_oversize_sparse_image(self):
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / 'oversize'
            with image.open('wb') as output:
                output.truncate(128 * 1024 * 1024 + 1)
            with self.assertRaisesRegex(RuntimeError, 'bounded regular file'):
                MODULE.binary_evidence(image)

    def test_existing_report_is_preserved_before_starting_any_product(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'prior.json'
            output.write_text('original evidence')
            with patch.object(sys, 'argv', ['accept-readonly-http.py', '--output', str(output)]), \
                    patch.object(MODULE.subprocess, 'run') as run, \
                    contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    MODULE.main()
            self.assertEqual(raised.exception.code, 2)
            self.assertEqual(output.read_text(), 'original evidence')
            run.assert_not_called()

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

    def test_token_preparation_cannot_issue_a_request_after_original_deadline(self):
        now = [100.0]
        minted = [0]

        def mint(key):
            minted[0] += 1
            if minted[0] == 2:
                now[0] += 0.75
            return 'fixture-bearer'

        def read(*args, **kwargs):
            self.assertLess(now[0], 101.0, 'network request started after its original deadline')
            now[0] += 0.25
            return 200, {'result': {'structuredContent': {
                'scope_id': 'scope-a', 'data': {'items': [1]}}}}

        with patch.object(MODULE.time, 'monotonic', side_effect=lambda: now[0]), \
                patch.object(MODULE, 'token', side_effect=mint), \
                patch.object(MODULE, 'request', side_effect=read) as request:
            result = MODULE.soak_reads(1234, {}, 'key', 'scope-a', 1.0)
        request.assert_called_once()
        self.assertEqual(result['requests'], 1)
        self.assertEqual(result['elapsed_seconds'], 1.0)

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
        notes = getattr(original, '__notes__', [])
        self.assertTrue(any('soak_request_timing' in note for note in notes))

    def test_rejects_nonfinite_or_nonpositive_duration_before_request(self):
        for seconds in (0, -1, float('nan'), float('inf'), 86401):
            with self.subTest(seconds=seconds), patch.object(MODULE, 'request') as read:
                with self.assertRaises(ValueError):
                    MODULE.soak_reads(1234, {}, 'key', 'scope-a', seconds)
                read.assert_not_called()
