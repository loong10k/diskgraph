"""CI phase coverage must retain every workspace target and original concurrency."""
import re
import tomllib
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]

class WorkspaceCiCoverageTests(unittest.TestCase):
    def test_budget_failure_observations_keep_default_target_scheduling(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding='utf-8')
        match = re.search(r"(?m)^      - name: Observe original Windows FFI failure with default scheduling\n(?P<body>.*?)(?=^      - |\Z)", workflow, re.S | re.M)
        self.assertIsNotNone(match)
        body = match.group('body')
        self.assertIn("steps.workspace_foundation_tests.outcome == 'failure'", body)
        self.assertIn("DISKGRAPH_QUERY_DIAGNOSTICS: '1'", body)
        self.assertIn('cargo test -p diskgraph-ffi --lib --locked -- --nocapture', body)
        self.assertNotIn('--test-threads', body)
        self.assertNotIn('--exact', body)
        self.assertIn('exit $LASTEXITCODE', body)
        start = workflow.index("command = ['cargo', 'test', '-p', 'diskgraph-engine', '--test',")
        end = workflow.index('raise SystemExit(1 if failed else 0)', start)
        whole_target = workflow[start:end]
        self.assertIn("'history_preparation_budget'", whole_target)
        self.assertNotIn('--test-threads', whole_target)
        self.assertNotIn('--exact', whole_target)
        self.assertIn("'replaces_original_failure': False", whole_target)

    def test_gnu_package_builds_and_worker_receipt_share_the_advertised_baseline(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding='utf-8')
        def step(name):
            match = re.search(r"(?m)^      - name: " + re.escape(name) +
                              r"\n(?P<body>.*?)(?=^      - |\Z)", workflow, re.S | re.M)
            self.assertIsNotNone(match, name)
            return match.group("body")

        setup = step("Prepare isolated GNU compatibility compiler")
        self.assertIn("if: runner.os == 'Linux'", setup)
        self.assertIn('python -m venv "$RUNNER_TEMP/diskgraph-gnu-builder"', setup)
        for pin in ('cargo-zigbuild==0.23.4', 'ziglang==0.16.0'):
            self.assertIn(pin, setup)
        self.assertIn('CARGO_ZIGBUILD_PYTHON_PATH=', setup)
        for name in ("Build GNU release binaries against original glibc baseline",
                     "Build previous GNU CLI against original glibc baseline",
                     "Bind Linux package acceptance to actual release worker"):
            body = step(name)
            self.assertIn("if: runner.os == 'Linux'", body)
            self.assertIn('cargo zigbuild --release --locked', body)
            self.assertIn('--target ${{ matrix.target }}.2.17', body)
            self.assertNotIn('cargo build ', body)
        for name in ("Build current release binaries", "Build previous CLI for isolated upgrade and rollback"):
            self.assertIn("if: runner.os != 'Linux'", step(name))
        self.assertLess(workflow.index('Prepare isolated GNU compatibility compiler'),
                        workflow.index('Build GNU release binaries against original glibc baseline'))
        self.assertIn('Verify actual GNU package ABI baseline', workflow)

    def test_macos_failure_isolation_includes_the_actual_settings_matrix_failure(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding='utf-8')
        match = re.search(r"(?m)^        id: macos_failure_isolation\n(?P<body>.*?)(?=^      - |\Z)", workflow, re.S | re.M)
        self.assertIsNotNone(match)
        body = match.group("body")
        self.assertIn("'every_recorded_scan_setting_difference_refuses_growth_and_changes'", body)
        self.assertIn("DISKGRAPH_QUERY_DIAGNOSTICS: '1'", body)
        self.assertIn("original workspace failure remains", body)

    def test_phase_coverage_survives_legacy_host_encoding(self):
        original_open = Path.open

        def legacy_host_open(path, *args, **kwargs):
            if kwargs.get('encoding') in (None, 'locale'):
                kwargs['encoding'] = 'cp1252'
            return original_open(path, *args, **kwargs)

        # 使用真实UTF-8工作流，仅模拟Windows的默认文本codec，不替换内容或覆盖结果。
        with mock.patch.object(Path, 'open', legacy_host_open):
            self.test_bounded_phases_cover_every_workspace_package_once()

    def test_bounded_phases_cover_every_workspace_package_once(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding='utf-8')
        phases = []
        for name in ("workspace_foundation_tests", "workspace_tests"):
            match = re.search(r"(?m)^      - name: [^\n]+\n        id: " + name + r"\n(?P<body>.*?)(?=^      - |\Z)", workflow, re.S | re.M)
            self.assertIsNotNone(match, name)
            body = match.group("body")
            self.assertIn("timeout-minutes: 30", body)
            self.assertNotIn("continue-on-error", body)
            self.assertNotIn("--test-threads", body)
            for flag in ("--all-targets", "--locked", "--no-fail-fast", "--nocapture"):
                self.assertIn(flag, body)
            phases.extend(re.findall(r"-p (diskgraph-[a-z-]+)", body))
        members = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding='utf-8'))["workspace"]["members"]
        expected = [tomllib.loads((ROOT / member / "Cargo.toml").read_text(encoding='utf-8'))["package"]["name"] for member in members]
        self.assertCountEqual(phases, expected)
        self.assertEqual(len(phases), len(set(phases)))
        # A foundation failure must not hide the entry-point suite.
        self.assertIn("steps.workspace_foundation_tests.outcome == 'failure'", workflow)

if __name__ == "__main__":
    unittest.main()
