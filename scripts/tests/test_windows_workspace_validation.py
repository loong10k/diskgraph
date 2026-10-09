"""台式机完整验收分段契约；模拟 owner 不等于原生 Windows 验收。"""
import importlib.util
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
SPEC = importlib.util.spec_from_file_location('workspace_validation', ROOT / 'scripts/run_windows_workspace_validation.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class WorkspaceValidationTests(unittest.TestCase):
    def exercise(self, results):
        calls = []
        def owner(command, **options):
            calls.append((command, options))
            result = results[len(calls) - 1]
            if isinstance(result, BaseException):
                raise result
            return subprocess.CompletedProcess(command, result, None, None)
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / 'receipt'
            receipt = MODULE.run_phases(output, owner, {'fixture': True})
            saved = json.loads((output / 'receipt.json').read_text(encoding='utf-8'))
            self.assertEqual(receipt, saved)
        return receipt, calls

    def test_real_ci_phase_coverage_and_original_arguments(self):
        workflow = (ROOT / '.github/workflows/ci.yml').read_text(encoding='utf-8')
        covered = []
        for phase, packages in MODULE.PHASES:
            identifier = 'workspace_foundation_tests' if phase == 'foundation' else 'workspace_tests'
            match = re.search(r'^      - name: [^\n]+\n        id: ' + identifier + r'\n(.*?)(?=^      - |\Z)', workflow, re.M | re.S)
            self.assertIsNotNone(match)
            self.assertEqual(list(packages), re.findall(r'-p (diskgraph-[a-z-]+)', match[1]))
            command = MODULE.phase_command(phase)
            for flag in ('--all-targets', '--locked', '--no-fail-fast', '--nocapture'):
                self.assertIn(flag, command)
            self.assertNotIn('--test-threads', command)
            self.assertNotIn('--workspace', command)
            covered.extend(packages)
        members = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['members']
        expected = [tomllib.loads((ROOT / m / 'Cargo.toml').read_text(encoding='utf-8'))['package']['name'] for m in members]
        self.assertCountEqual(expected, covered)
        self.assertEqual(len(covered), len(set(covered)))

    def test_both_phases_have_independent_original_deadlines(self):
        receipt, calls = self.exercise([0, 0])
        self.assertEqual(len(calls), 2)
        self.assertEqual([c[1]['timeout'] for c in calls], [1800, 1800])
        self.assertTrue(receipt['passed'])
        self.assertTrue(all(r['job_retirement_confirmed'] for r in receipt['phases']))

    def test_first_failure_does_not_hide_entry_phase(self):
        receipt, calls = self.exercise([101, 0])
        self.assertEqual(len(calls), 2)
        self.assertFalse(receipt['passed'])
        self.assertEqual([r['exit_code'] for r in receipt['phases']], [101, 0])

    def test_retired_timeout_is_recorded_and_next_phase_runs(self):
        receipt, calls = self.exercise([subprocess.TimeoutExpired(['phase'], 1800), 0])
        self.assertEqual(len(calls), 2)
        self.assertFalse(receipt['passed'])
        self.assertTrue(receipt['phases'][0]['timed_out'])
        self.assertIsNone(receipt['phases'][0]['exit_code'])

    def test_uncertain_retirement_prevents_any_next_phase(self):
        failure = subprocess.TimeoutExpired(['phase'], 1800)
        failure.add_note('Job retirement or pipe drain unconfirmed: still active')
        receipt, calls = self.exercise([failure])
        self.assertEqual(len(calls), 1)
        self.assertFalse(receipt['passed'])
        self.assertFalse(receipt['phases'][0]['job_retirement_confirmed'])

    def test_startup_or_owner_error_refuses_complete_pass(self):
        receipt, calls = self.exercise([OSError('owner unavailable')])
        self.assertEqual(len(calls), 1)
        self.assertFalse(receipt['passed'])
        self.assertFalse(receipt['phases'][0]['job_retirement_confirmed'])

    def test_output_directory_is_exclusive(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileExistsError):
                MODULE.run_phases(pathlib.Path(directory), lambda *a, **k: None, {})


if __name__ == '__main__':
    unittest.main()
