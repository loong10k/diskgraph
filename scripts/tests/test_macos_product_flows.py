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
        self.assertEqual(flows.node_facts(value, 3, 15), node)
        with self.assertRaises(RuntimeError):
            flows.node_facts(value, 4, 15)

    def test_unknown_size_and_missing_counts_are_rejected(self):
        for node in ({"files": "3", "directories": "2", "subtree_bytes": None},
                     {"files": None, "directories": "2", "subtree_bytes": "15"},
                     {"files": "3", "subtree_bytes": "15"}):
            with self.assertRaises((RuntimeError, TypeError, KeyError)):
                flows.node_facts({"api_version": 2, "ok": True, "data": {"node": node}}, 3, 15)

    def test_incorrect_known_bytes_are_rejected(self):
        for actual in ("0", "999", "14", "16"):
            value = {"api_version": 2, "ok": True, "data": {"node":
                     {"files": "3", "directories": "2", "subtree_bytes": actual}}}
            with self.assertRaises(RuntimeError):
                flows.node_facts(value, 3, 15)

    def test_empty_error_object_is_not_success(self):
        with self.assertRaises(RuntimeError):
            flows.envelope({"api_version": 2, "ok": True, "data": {}, "error": {}})

    def test_rpc_requires_version_exact_integer_id_and_success_result(self):
        valid = {"jsonrpc": "2.0", "id": 1, "result": {}}
        self.assertEqual(flows.rpc_result(valid, 1), {})
        for value in ({"id": 1, "result": {}},
                      {"jsonrpc": "1.0", "id": 1, "result": {}},
                      {"jsonrpc": "2.0", "id": True, "result": {}},
                      {"jsonrpc": "2.0", "id": 2, "result": {}},
                      {**valid, "error": {}},
                      {"jsonrpc": "2.0", "id": 1}):
            with self.assertRaises(RuntimeError):
                flows.rpc_result(value, 1)
