"""历史容量回放保持真实创建路径，拒绝不完整的依赖剔除。"""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from qualify_windows_capacity_replay import adapt_support, SUPPORT


class CapacityReplayTests(unittest.TestCase):
    def test_current_summary_and_budget_are_removed_together_only_in_replay(self):
        root = Path(__file__).resolve().parents[2]
        current = {name: (root / name).read_bytes() for name in SUPPORT}
        budget = "crates/diskgraph-engine/src/live_evidence/probe_budget.rs"
        current[budget] = (root / budget).read_bytes()
        result = adapt_support(current)
        self.assertNotIn(b"mod git_private_write_summary;", result[SUPPORT[3]])
        self.assertNotIn(b"_write_summary", result[budget])
        expected = current[budget].replace(
            b"    #[cfg(all(test, windows))]\n"
            b"    _write_summary: Option<super::git_private_write_summary::GitPrivateWriteSummary>,\n", b""
        ).replace(
            b"            #[cfg(all(test, windows))]\n"
            b"            _write_summary: super::git_private_write_summary::GitPrivateWriteSummary::new(),\n", b""
        )
        self.assertEqual(result[budget], expected)

    def test_removes_only_unreachable_creation_recovery_calls(self):
        block = (b"        if let Some(capacity) = self.capacity.as_mut() {\n"
                 b"            capacity.recover_created_entry()?;\n        }\n")
        owner = b"before\n" + block + b"cleanup_until(real);\n" + b"    " + block.replace(b"\n", b"\n    ").rstrip() + b"\nend\n"
        modules = (b"#[cfg(windows)]\nmod git_private_created_entry;\n"
                   b"#[cfg(all(test, windows))]\nmod git_private_write_profile;\n"
                   b"mod current_tests;\n")
        result = adapt_support({SUPPORT[2]: owner, SUPPORT[3]: modules})
        self.assertNotIn(b"recover_created_entry", result[SUPPORT[2]])
        self.assertIn(b"cleanup_until(real);", result[SUPPORT[2]])
        self.assertEqual(result[SUPPORT[3]], b"mod current_tests;\n")

    def test_rejects_missing_owner_call(self):
        with self.assertRaises(RuntimeError):
            adapt_support({SUPPORT[2]: b"changed owner", SUPPORT[3]: b"changed modules"})


if __name__ == "__main__":
    unittest.main()
