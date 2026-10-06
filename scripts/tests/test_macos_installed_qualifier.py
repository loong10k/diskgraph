import hashlib
import importlib.util
import io
import json
import tarfile
import tempfile
import tomllib
import subprocess
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "qualify_macos_installed_worker.py"
spec = importlib.util.spec_from_file_location("qualifier", SCRIPT)
qualifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualifier)


class MacosInstalledQualifierTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.checkout = Path(self.temp.name)
        self.directory = self.checkout / qualifier.CANDIDATE
        self.directory.mkdir(parents=True)

    def candidate(self, records, declared=None):
        archive = self.directory / "candidate.tar.gz"
        with tarfile.open(archive, "w:gz") as out:
            for name, content, kind in records:
                info = tarfile.TarInfo(name)
                if kind == "file":
                    info.size = len(content)
                    out.addfile(info, io.BytesIO(content))
                else:
                    info.type = tarfile.SYMTYPE
                    info.linkname = "/tmp/escaped"
                    out.addfile(info)
        sources = declared or {name: hashlib.sha256(content).hexdigest() for name, content, _ in records}
        manifest = {"schema_version": 1, "archive_sha256": qualifier.digest(archive), "sources": sources}
        (self.directory / "manifest.json").write_text(json.dumps(manifest))

    def test_real_committed_candidate_has_complete_permitted_inventory(self):
        source = SCRIPT.parent.parent / qualifier.CANDIDATE
        for name in ["candidate.tar.gz", "manifest.json"]:
            (self.directory / name).write_bytes((source / name).read_bytes())
        manifest = qualifier.mount(self.checkout, allow_products=True)
        self.assertEqual(len(manifest["sources"]), 600)
        self.assertEqual(manifest["native_child_parallel_tests_required"], 41)
        self.assertEqual(len(manifest["ordinary_cases"]), 6)
        self.assertEqual(manifest["protocol_cases"], [qualifier.PROTOCOL_CASE, qualifier.BUDGET_FIXTURE_CASE])
        self.assertEqual(manifest["fixture_features"], ["macos_native_scan_candidate"])
        self.assertEqual(manifest["product_flows"], ["cli_init_node", "mcp_stdio_index_status_node", "cli_observes_mcp_revision"])
        for name, expected in manifest["sources"].items():
            self.assertEqual(qualifier.digest(self.checkout / name), expected)

    def test_frozen_lock_matches_explicit_mounted_product_dependencies(self):
        source = SCRIPT.parent.parent / qualifier.CANDIDATE
        for name in ["candidate.tar.gz", "manifest.json"]:
            (self.directory / name).write_bytes((source / name).read_bytes())
        qualifier.mount(self.checkout, allow_products=True)
        packages = {p["name"]: p for p in tomllib.loads((self.checkout / "Cargo.lock").read_text())["package"]}
        for package in ("diskgraph-cli", "diskgraph-mcp"):
            # 产品包现在显式冻结并挂载；锁依赖必须与同一冻结清单一致，不能混用HEAD旧清单。
            manifest = tomllib.loads((self.checkout / "crates" / package / "Cargo.toml").read_text())
            expected = {name for group in ("dependencies", "dev-dependencies", "build-dependencies")
                        for name in manifest.get(group, {}) if name.startswith("diskgraph-")}
            actual = {name for name in packages[package]["dependencies"] if name.startswith("diskgraph-")}
            self.assertEqual(actual, expected, package)

    def test_regular_frozen_sources_mount_and_match(self):
        self.candidate([("crates/diskgraph-engine/src/frozen.rs", b"source", "file")])
        qualifier.mount(self.checkout)
        self.assertEqual((self.checkout / "crates/diskgraph-engine/src/frozen.rs").read_bytes(), b"source")

    def test_source_escape_and_non_source_inputs_are_rejected(self):
        for name in ["/tmp/escaped", "crates/diskgraph-engine/src/../../escaped", ".git/config", "crates/diskgraph-engine/src/.git/config"]:
            self.candidate([(name, b"bad", "file")])
            with self.assertRaises(ValueError):
                qualifier.mount(self.checkout)

    def test_symbolic_archive_member_is_rejected(self):
        self.candidate([("crates/diskgraph-engine/src/frozen.rs", b"", "symlink")])
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)

    def test_digest_failure_never_partially_replaces_existing_source(self):
        a = "crates/diskgraph-engine/src/a.rs"
        b = "crates/diskgraph-engine/src/b.rs"
        existing = self.checkout / a
        existing.parent.mkdir(parents=True, exist_ok=True)
        existing.write_bytes(b"original")
        self.candidate([(a, b"changed", "file"), (b, b"bad", "file")], {a: hashlib.sha256(b"changed").hexdigest(), b: "0" * 64})
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)
        self.assertEqual(existing.read_bytes(), b"original")
        self.assertFalse((self.checkout / b).exists())

    def test_duplicate_members_and_changed_archive_are_rejected(self):
        name = "crates/diskgraph-engine/src/a.rs"
        self.candidate([(name, b"a", "file"), (name, b"a", "file")])
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)
        self.candidate([(name, b"a", "file")])
        with (self.directory / "candidate.tar.gz").open("ab") as out:
            out.write(b"tampered")
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)

    def test_product_sources_require_explicit_mount_scope(self):
        name = "crates/diskgraph-cli/src/main.rs"
        self.candidate([(name, b"fn main() {}", "file")])
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)
        result = qualifier.mount(self.checkout, allow_products=True)
        self.assertEqual((self.checkout / name).read_bytes(), b"fn main() {}")
        self.assertIn(name, result["sources"])

    def test_destination_parent_symlink_is_rejected(self):
        outside = self.checkout / "outside"
        outside.mkdir()
        parent = self.checkout / "crates/diskgraph-engine"
        parent.mkdir(parents=True, exist_ok=True)
        (parent / "src").symlink_to(outside, target_is_directory=True)
        self.candidate([("crates/diskgraph-engine/src/a.rs", b"a", "file")])
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)
        self.assertFalse((outside / "a.rs").exists())

    def test_full_cli_regression_requires_original_cases_and_counts(self):
        valid = "\n".join("test " + case + " ... ok" for case in qualifier.CLI_SCAN_REGRESSION_CASES)
        valid += "\ntest result: ok. 67 passed; 0 failed; 0 ignored;"
        qualifier.check_cli_regression(valid)
        for invalid in (valid.replace("67 passed", "0 passed"), valid.replace("0 ignored", "1 ignored"),
                        valid.replace(" ... ok", " ... ignored", 1)):
            with self.assertRaises(RuntimeError):
                qualifier.check_cli_regression(invalid)

    def test_full_mcp_regression_requires_original_cases_and_counts(self):
        valid = "\n".join("test " + case + " ... ok" for case in qualifier.MCP_SCAN_REGRESSION_CASES)
        valid += "\ntest result: ok. 162 passed; 0 failed; 0 ignored;"
        qualifier.check_mcp_regression(valid)
        for invalid in (valid.replace("162 passed", "0 passed"), valid.replace("0 ignored", "1 ignored"),
                        valid.replace(" ... ok", " ... ignored", 1)):
            with self.assertRaises(RuntimeError):
                qualifier.check_mcp_regression(invalid)


if __name__ == "__main__":
    unittest.main()


class NativeBirthParallelGuardTests(unittest.TestCase):
    def test_requires_actual_named_cases_and_complete_parallel_subset(self):
        output = "\n".join("test " + case + " ... ok" for case in qualifier.NATIVE_BIRTH_REGRESSION_CASES)
        output += "\ntest native_child::unix_leader_tests::nonpositive_native_pid_isolated_fixture ... ignored, invoked by the real isolated ownership regression\nDG_NONPOSITIVE_WAIT_FIXTURE_VERIFIED pid=0,-1,-42\ntest result: ok. 41 passed; 0 failed; 1 ignored; 0 measured; 522 filtered out;"
        qualifier.check_native_birth_regression(output)
        for invalid in ("", output.replace("41 passed", "0 passed"),
                        output.replace("1 ignored", "2 ignored"),
                        output.replace("0 failed", "1 failed"),
                        output.replace(qualifier.NATIVE_BIRTH_REGRESSION_CASES[0], "fake_case"),
                        output.replace(qualifier.NATIVE_BIRTH_REGRESSION_CASES[-1], "fake_wait_case"),
                        output.replace("DG_NONPOSITIVE_WAIT_FIXTURE_VERIFIED", "missing_fixture"),
                        output.replace("41 passed", "40 passed")):
            with self.assertRaises(RuntimeError):
                qualifier.check_native_birth_regression(invalid)


class FullEngineRegressionGuardTests(unittest.TestCase):
    def test_full_engine_requires_exact_inventory_zero_failures_and_no_filter(self):
        stdout = "test result: ok. 552 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out;"
        qualifier.check_full_engine_regression(stdout, 559, 7)
        for out, total, ignored in ((stdout, 558, 7), (stdout, 559, 8),
                                   (stdout.replace("0 failed", "1 failed"), 559, 7),
                                   (stdout.replace("0 filtered", "1 filtered"), 559, 7),
                                   ("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;", 1, 0)):
            with self.assertRaises(RuntimeError):
                qualifier.check_full_engine_regression(out, total, ignored)
