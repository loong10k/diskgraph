#!/usr/bin/env python3
"""memfd 资格脚本的纯装配/错误顺序测试；不代替实际 ELF 或 namespace 验收。"""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("memfd_qualification", ROOT / "scripts/qualify-linux-memfd-execution.py")
QUALIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(QUALIFIER)


class MemfdAssemblyTests(unittest.TestCase):
    """消费实际冻结材料装配临时目录，不运行 Git、Cargo 或 cc。"""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.checkout = Path(self.temporary.name)
        self.manifest = json.loads((ROOT / QUALIFIER.MANIFEST).read_text())
        self.atomic = json.loads((ROOT / QUALIFIER.ATOMIC_MANIFEST).read_text())
        paths = [source["path"] for source in self.manifest["sources"] + self.atomic["sources"]]
        tool = self.atomic["qualification_tooling"]
        outer = self.atomic["required_outer_supervisor"]
        paths += [binding["path"] for binding in [self.manifest["qualifier"], self.manifest["unit_tests"],
                  tool["qualifier"], tool["unit_tests"], tool["shared_support"],
                  outer["source"], outer["unit_tests"], outer["workflow"]]]
        paths.append(self.manifest["profile_helper"]["path"])
        paths.append(QUALIFIER.ATOMIC_MANIFEST)
        for name in paths:
            target = self.checkout / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        native = self.checkout / QUALIFIER.NATIVE
        native.mkdir(parents=True)
        before = next(source for source in self.atomic["sources"] if source["role"] == "baseline_before")
        shutil.copyfile(self.checkout / before["path"], native / "unix_child.rs")
        (native / "mod.rs").write_text("#[cfg(unix)]\nmod unix_child;\n")
        (self.checkout / "crates/diskgraph-engine/Cargo.toml").write_text('[package]\nname="diskgraph-engine"\nversion="0.1.0"\n')
        (self.checkout / "Cargo.lock").write_text('version = 4\n[[package]]\nname = "cc"\nversion = "1.2.0"\n[[package]]\nname = "diskgraph-engine"\nversion = "0.1.0"\ndependencies = [\n "libc",\n]\n')

    def test_actual_ten_sources_mount_without_flags_or_test_changes(self):
        atomic = QUALIFIER.validate_profile(self.checkout, self.manifest, ROOT / "scripts/qualify-linux-memfd-execution.py")
        result = QUALIFIER.mount_profile(self.checkout, self.manifest, atomic)
        self.assertEqual(len(result["memfd_declarations"]), 9)
        for source in self.manifest["sources"]:
            if source.get("module"):
                actual = self.checkout / (QUALIFIER.NATIVE + source["module"] + ".rs")
                self.assertEqual(QUALIFIER.ATOMIC.digest(actual), source["sha256"])
        self.assertEqual(atomic, self.atomic)
        self.assertEqual(QUALIFIER.ATOMIC.digest(self.checkout / QUALIFIER.ATOMIC_MANIFEST), self.manifest["atomic_manifest_sha256"])

    def test_late_corrupt_source_rejects_before_mount(self):
        source = self.manifest["sources"][-1]
        (self.checkout / source["path"]).write_bytes(b"corrupt fixture")
        native = self.checkout / (QUALIFIER.NATIVE + "mod.rs")
        before = native.read_bytes()
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            QUALIFIER.mount_profile(self.checkout, self.manifest, self.atomic)
        self.assertEqual(native.read_bytes(), before)
        self.assertFalse((self.checkout / (QUALIFIER.NATIVE + "linux_atomic_launcher.rs")).exists())

    def test_groups_keep_existing_two_one_three_not_equalized(self):
        manifest = copy.deepcopy(self.manifest)
        manifest["test_groups"][1]["names"].append(manifest["test_groups"][2]["names"].pop())
        with self.assertRaisesRegex(ValueError, "actual2/1/3"):
            QUALIFIER.validate_additional_sources(self.checkout, manifest)

    def test_policy_fixture_and_source_paths_cannot_escape(self):
        manifest = copy.deepcopy(self.manifest)
        manifest["policy_fixture"] = "../different.c"
        with self.assertRaisesRegex(ValueError, "exact admitted C source"):
            QUALIFIER.validate_additional_sources(self.checkout, manifest)
        manifest = copy.deepcopy(self.manifest)
        manifest["sources"][0]["path"] = "../different.rs"
        with self.assertRaisesRegex(ValueError, "candidate path"):
            QUALIFIER.validate_additional_sources(self.checkout, manifest)

    def test_active_or_archived_profile_binding_drift_is_denied(self):
        different = self.checkout / "different.py"
        different.write_bytes(b"different active script")
        with self.assertRaisesRegex(ValueError, "running memfd qualifier"):
            QUALIFIER.validate_profile(self.checkout, self.manifest, different)
        (self.checkout / QUALIFIER.ATOMIC_MANIFEST).write_text("{}")
        with self.assertRaisesRegex(ValueError, "atomic manifest differs"):
            QUALIFIER.validate_profile(self.checkout, self.manifest, ROOT / "scripts/qualify-linux-memfd-execution.py")


    def test_archived_profile_helper_drift_is_denied_before_assembly(self):
        helper = self.checkout / self.manifest["profile_helper"]["path"]
        helper.write_bytes(b"altered root QA lifecycle implementation")
        native = self.checkout / (QUALIFIER.NATIVE + "mod.rs")
        before = native.read_bytes()
        with self.assertRaisesRegex(ValueError, "profile helper.*digest mismatch"):
            QUALIFIER.validate_profile(self.checkout, self.manifest,
                                       ROOT / "scripts/qualify-linux-memfd-execution.py")
        self.assertEqual(native.read_bytes(), before)


class MemfdResultTests(unittest.TestCase):
    """仅驱动 Python 的原返回分类；人为 summary 不声称是真实原生结果。"""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.output = Path(self.temporary.name)
        self.groups = json.loads((ROOT / QUALIFIER.MANIFEST).read_text())["test_groups"]
        self.receipt = {}
        self.errors = {}
        self.support = type("Support", (), {"secondary": lambda _, receipt, key, error: self.errors.update({key: error})})()

    def invoke(self, behavior):
        with patch.object(QUALIFIER.ATOMIC, "run_step", side_effect=behavior):
            QUALIFIER.run_cases(self.output, self.output, self.receipt, {}, self.support, self.groups)

    def write_summary(self, name, passed, failed, ignored=0):
        path = self.output / (name + ".log")
        status = "FAILED" if failed else "ok"
        path.write_text(f"test result: {status}. {passed} passed; {failed} failed; {ignored} ignored;\n")
        return path

    def test_all_exact_filters_are_serially_consumed(self):
        seen = []

        def step(receipt, output, name, command, *rest):
            group = self.groups[len(seen)]
            seen.append(command)
            self.assertIn(group["filter"], command)
            self.assertEqual(command[-2:], ["--nocapture", "--test-threads=1"])
            return self.write_summary(name, len(group["names"]), 0)

        self.invoke(step)
        self.assertEqual(len(seen), 3)
        self.assertEqual(sum(item["passed"] for item in self.receipt["native_results"]), 6)

    def test_first_actual_failure_preserved_and_later_filters_still_run(self):
        seen = []
        failure = subprocess.CalledProcessError(101, ["first-native-target"])

        def step(receipt, output, name, command, *rest):
            group = self.groups[len(seen)]
            seen.append(name)
            if len(seen) == 1:
                self.write_summary(name, 1, 1)
                raise failure
            return self.write_summary(name, len(group["names"]), 0)

        with self.assertRaises(subprocess.CalledProcessError) as caught:
            self.invoke(step)
        self.assertIs(caught.exception, failure)
        self.assertEqual(len(seen), 3)
        self.assertEqual(len(self.receipt["native_results"]), 3)

    def test_ignored_or_incomplete_summary_never_passes(self):
        seen = []

        def step(receipt, output, name, command, *rest):
            group = self.groups[len(seen)]
            seen.append(name)
            return self.write_summary(name, len(group["names"]), 0, ignored=1)

        with self.assertRaisesRegex(ValueError, "case count/exit inconsistent"):
            self.invoke(step)
        self.assertEqual(len(seen), 3)

    def test_later_timeout_does_not_cover_first_target_failure(self):
        seen = []
        failure = subprocess.CalledProcessError(101, ["first-native-target"])
        timeout = subprocess.TimeoutExpired(["later-native-target"], 1200)

        def step(receipt, output, name, command, *rest):
            seen.append(name)
            if len(seen) == 1:
                self.write_summary(name, 1, 1)
                raise failure
            raise timeout

        with self.assertRaises(subprocess.CalledProcessError) as caught:
            self.invoke(step)
        self.assertIs(caught.exception, failure)
        self.assertEqual(len(seen), 2)
        self.assertIn(timeout, self.errors.values())

    def test_log_read_error_cannot_cover_actual_target_failure(self):
        failure = subprocess.CalledProcessError(101, ["first-native-target"])
        read_error = OSError("native log read failed")

        def step(receipt, output, name, command, *rest):
            self.write_summary(name, 1, 1)
            raise failure

        with patch.object(Path, "read_bytes", side_effect=read_error):
            with self.assertRaises(subprocess.CalledProcessError) as caught:
                self.invoke(step)
        self.assertIs(caught.exception, failure)
        self.assertIn(read_error, self.errors.values())
        self.assertEqual(self.receipt["native_results"][0]["qualification"], "native log unavailable")

    def test_log_read_error_after_success_is_not_swallowed(self):
        read_error = OSError("native log read failed")

        def step(receipt, output, name, command, *rest):
            return self.write_summary(name, 2, 0)

        with patch.object(Path, "read_bytes", side_effect=read_error):
            with self.assertRaises(OSError) as caught:
                self.invoke(step)
        self.assertIs(caught.exception, read_error)
        self.assertFalse(self.errors)


if __name__ == "__main__":
    unittest.main()
