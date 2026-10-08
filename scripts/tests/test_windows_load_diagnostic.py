"""诊断不能替代正式200k验收，失败保留原异常及阶段证据。"""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location('windows_load_diagnostic', SCRIPTS / 'diagnose_windows_load.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class DiagnosticTests(unittest.TestCase):
    def exercise(self, outcome):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bins = root / 'bin'
            bins.mkdir()
            for name in ('diskgraph.exe', 'diskgraph-mcp.exe', 'diskgraph-scan-worker.exe'):
                (bins / name).write_bytes(b'controlled test image')
            output = root / 'diagnostic'
            with patch.object(MODULE, 'run_owned', side_effect=outcome if isinstance(outcome, Exception) else None,
                              return_value=outcome) as run:
                try:
                    result = MODULE.run(bins, output)
                except Exception as error:
                    result = error
            return result, json.loads((output / 'receipt.json').read_text()), run.call_args

    def test_complete_observation_never_claims_acceptance_and_keeps_full_workload(self):
        report = {'files': 200000, 'indexed_nodes': 200001, 'queries': 32,
                  'concurrent_clients': 4, 'passed': 6, 'total': 6,
                  'checks': {str(index): True for index in range(6)}}
        result, receipt, call = self.exercise(subprocess.CompletedProcess([], 0, json.dumps(report), 'phase evidence'))
        self.assertFalse(receipt['production_acceptance'])
        self.assertEqual(receipt['status'], 'complete')
        self.assertIn('200000', call.args[0])
        self.assertEqual(call.kwargs['timeout'], 900)
        self.assertEqual(receipt['formal_timeout_seconds'], 300)
        self.assertEqual(len(receipt['binary_sha256']), 3)
        self.assertEqual(len(receipt['source_sha256']), 3)
        self.assertFalse(receipt['clean_environment_confirmed'])
        self.assertEqual(receipt['inner_cli_timeout_seconds'], 300)
        self.assertGreaterEqual(receipt['elapsed_seconds'], 0)

    def test_timeout_is_preserved_and_receipt_stays_failed(self):
        failure = subprocess.TimeoutExpired(['diagnostic'], 900, stderr=b'index begin')
        result, receipt, _ = self.exercise(failure)
        self.assertIs(result, failure)
        self.assertEqual(receipt['status'], 'failed')
        self.assertFalse(receipt['production_acceptance'])

    def test_nonzero_command_exit_remains_failed(self):
        result, receipt, _ = self.exercise(subprocess.CompletedProcess([], 10, 'failed stdout', 'failed stderr'))
        self.assertIsInstance(result, RuntimeError)
        self.assertEqual(receipt['status'], 'failed')
        self.assertNotIn('observed_load', receipt)

    def test_partial_result_is_not_a_complete_diagnostic(self):
        result, receipt, _ = self.exercise(subprocess.CompletedProcess([], 0, '{"files": 2}', ''))
        self.assertIsInstance(result, RuntimeError)
        self.assertEqual(receipt['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
