"""CI phase coverage must retain every workspace target and original concurrency."""
import re
import tomllib
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]

class WorkspaceCiCoverageTests(unittest.TestCase):
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
