"""CI phase coverage must retain every workspace target and original concurrency."""
import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

class WorkspaceCiCoverageTests(unittest.TestCase):
    def test_bounded_phases_cover_every_workspace_package_once(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
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
        members = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["members"]
        expected = [tomllib.loads((ROOT / member / "Cargo.toml").read_text())["package"]["name"] for member in members]
        self.assertCountEqual(phases, expected)
        self.assertEqual(len(phases), len(set(phases)))
        # A foundation failure must not hide the entry-point suite.
        self.assertIn("steps.workspace_foundation_tests.outcome == 'failure'", workflow)

if __name__ == "__main__":
    unittest.main()
