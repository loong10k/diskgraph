"""产品验收的响应拒绝边界；不替代实际 CLI/MCP 进程验收。"""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "product_flows", Path(__file__).resolve().parents[1] / "qualify_macos_product_flows.py")
flows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(flows)


class MacosProductResponseTests(unittest.TestCase):
    def test_errors_and_missing_data_are_never_product_success(self):
        for value in ({}, {"api_version": 1, "ok": True, "data": {}},
                      {"api_version": 2, "ok": False, "data": {}},
                      {"api_version": 2, "ok": True, "data": None},
                      {"api_version": 2, "ok": True, "data": {}, "error": {"code": "unsupported"}}):
            with self.assertRaises(RuntimeError):
                flows.envelope(value)

    def test_wire_decimal_counts_are_checked_against_real_fixture(self):
        node = {"files": "3", "directories": "2", "subtree_bytes": "15"}
        value = {"api_version": 2, "ok": True, "data": {"node": node}}
        self.assertEqual(flows.node_facts(value, 3), node)
        with self.assertRaises(RuntimeError):
            flows.node_facts(value, 4)

    def test_unknown_size_and_missing_counts_are_rejected(self):
        for node in ({"files": "3", "directories": "2", "subtree_bytes": None},
                     {"files": None, "directories": "2", "subtree_bytes": "15"},
                     {"files": "3", "subtree_bytes": "15"}):
            with self.assertRaises((RuntimeError, TypeError, KeyError)):
                flows.node_facts({"api_version": 2, "ok": True, "data": {"node": node}}, 3)
