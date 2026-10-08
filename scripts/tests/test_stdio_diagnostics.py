"""stdio验收保留失败分类，不泄露返回的正文、主体或任意错误文本。"""
import importlib.util
import json
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    'stdio_acceptance', Path(__file__).resolve().parents[1] / 'accept-readonly-stdio.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class StdioDiagnosticsTests(unittest.TestCase):
    def test_budget_failure_keeps_only_safe_codes(self):
        result = MODULE.dispatch_diagnostic({'error': {
            'code': -32000, 'message': '/private/file', 'data': {
                'business_code': 'budget_exceeded', 'principal': 'private-user'}}}, True)
        self.assertEqual(result, {'schema_valid': True, 'rpc_error': True,
                                 'rpc_error_code': -32000, 'business_code': 'budget_exceeded',
                                 'structured_ok': False})
        self.assertNotIn('private', json.dumps(result))

    def test_malformed_payload_does_not_enter_report_or_raise(self):
        for value in [None, [], {'result': 'private-body'}, {'error': 'private-error'},
                      {'error': {'code': '/private/error', 'data': {
                          'business_code': '/private/business'}}},
                      {'error': {'code': True, 'data': []}}]:
            with self.subTest(value=value):
                result = MODULE.dispatch_diagnostic(value, False)
                self.assertNotIn('private', json.dumps(result))
                self.assertEqual(result['schema_valid'], False)
                self.assertIsNone(result['rpc_error_code'])
                self.assertEqual(result['business_code'], 'unknown')

    def test_budget_phase_parser_rejects_private_or_unknown_fields(self):
        data = '\n'.join([
            'private path /example token-secret',
            'diskgraph: authorization_budget_phase=terminal_reader_open elapsed_us=1234',
            'diskgraph: authorization_budget_phase=private_path elapsed_us=5',
            'diskgraph: authorization_budget_phase=terminal_reader_open elapsed_us=5 secret',
        ])
        self.assertEqual(MODULE.budget_phase_diagnostics(data), [
            {'phase': 'terminal_reader_open', 'elapsed_us': 1234}])
        repeated = '\n'.join(['diskgraph: authorization_budget_phase=terminal_server_sql elapsed_us=5'] * 100)
        self.assertEqual(len(MODULE.budget_phase_diagnostics(repeated)), 32)

    def test_classification_does_not_turn_rpc_error_into_success(self):
        result = MODULE.dispatch_diagnostic({'result': {'structuredContent': {'ok': True}},
                                             'error': {'code': -32602}}, True)
        self.assertTrue(result['rpc_error'])
        self.assertTrue(result['structured_ok'])
        self.assertEqual(result['rpc_error_code'], -32602)


if __name__ == '__main__':
    unittest.main()
