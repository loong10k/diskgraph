"""诊断不能替代正式200k验收，失败保留原异常及阶段证据。"""
import importlib.util
import json
import os
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
    def test_explicit_parent_routes_real_child_temporary_directory(self):
        report = {'files': 200000, 'indexed_nodes': 200001, 'queries': 32,
                  'concurrent_clients': 4, 'passed': 6, 'total': 6,
                  'checks': {str(index): True for index in range(6)}}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bins = root / 'bin'
            bins.mkdir()
            for name in ('diskgraph.exe', 'diskgraph-mcp.exe', 'diskgraph-scan-worker.exe'):
                (bins / name).write_bytes(b'controlled test image')
            parent = root / 'selected workspace'
            parent.mkdir()
            observed = []

            def child(command, **arguments):
                self.assertIn('200000', command)
                self.assertEqual(arguments['timeout'], 900)
                result = subprocess.run(
                    [sys.executable, '-c',
                     'import tempfile;\nwith tempfile.TemporaryDirectory() as path: print(path)'],
                    env=arguments['env'], capture_output=True, text=True,
                    check=True, timeout=10,
                )
                observed.append(Path(result.stdout.strip()))
                return subprocess.CompletedProcess(command, 0, json.dumps(report), '')

            with patch.object(MODULE, 'run_owned', side_effect=child):
                receipt = MODULE.run(bins, root / 'output', workspace_parent=parent)
            self.assertEqual(observed[0].parent, parent.resolve())
            self.assertFalse(observed[0].exists())
            self.assertFalse(receipt['production_acceptance'])
            self.assertEqual(receipt['formal_timeout_seconds'], 300)
            self.assertEqual(receipt['workspace_parent'], str(parent.resolve()))

    def test_file_parent_refuses_before_worker_dispatch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            parent = root / 'not a directory'
            parent.write_bytes(b'unchanged')
            with patch.object(MODULE, 'run_owned') as run:
                with self.assertRaises(NotADirectoryError):
                    MODULE.run(root / 'bin', root / 'output', workspace_parent=parent)
            run.assert_not_called()
            self.assertEqual(parent.read_bytes(), b'unchanged')

    def test_default_preserves_inherited_workspace_environment(self):
        with patch.dict(os.environ, {'TEMP': 'inherited-temp', 'TMP': 'inherited-tmp'}):
            _, _, call = self.exercise(subprocess.CompletedProcess([], 10, '', ''))
        self.assertEqual(call.kwargs['env']['TEMP'], 'inherited-temp')
        self.assertEqual(call.kwargs['env']['TMP'], 'inherited-tmp')

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
