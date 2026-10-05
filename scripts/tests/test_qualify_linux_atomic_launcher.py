#!/usr/bin/env python3
"""仅验证实际资格脚本的装配与错误保真；不执行或模拟 Linux launcher 通过。"""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch
from contextlib import ExitStack


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "atomic_qualification", ROOT / "scripts/qualify-linux-atomic-launcher.py"
)
QUALIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(QUALIFIER)


class RecordedSupport:
    """只记录次要 Python 错误，保留原异常对象身份。"""

    def secondary(self, receipt, key, error):
        receipt[key] = error


class CloseFailure:
    """真实文件先关闭，再从 Python close 边界发出指定材料错误。"""

    def __init__(self, stream, error):
        self.stream = stream
        self.error = error

    def close(self):
        self.stream.close()
        raise self.error


class ProcessResult:
    """仅用于命令编排错误单测；没有创建或宣称回收原子进程。"""

    pid = 12345

    def __init__(self, error=None):
        self.error = error
        self.returncode = None
        self.stdout = None

    def wait(self, timeout):
        if self.error is not None:
            error, self.error = self.error, None
            raise error
        self.returncode = 0
        return 0


class QualificationAssemblyTests(unittest.TestCase):
    """用冻结的真实源装配临时副本，不调用 Git、Cargo、cc 或原生测试。"""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.checkout = Path(self.temporary.name)
        self.manifest = json.loads((ROOT / QUALIFIER.MANIFEST).read_text())
        for source in self.manifest["sources"]:
            target = self.checkout / source["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / source["path"], target)
        tooling = self.manifest["qualification_tooling"]
        supervisor = self.manifest["required_outer_supervisor"]
        bindings = [tooling["qualifier"], tooling["unit_tests"], tooling["shared_support"],
                    supervisor["source"], supervisor["unit_tests"], supervisor["workflow"]]
        bindings += [{"path": path} for path in (
            "scripts/native_pid_namespace_restore.py",
            "scripts/tests/test_native_pid_namespace_restore.py",
            "scripts/tests/test_native_pid_namespace_restore_errors.py")]
        bindings += self.manifest["policy_stage_tooling"] + self.manifest["policy_stage_tests"]
        for binding in bindings:
            target = self.checkout / binding["path"]
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / binding["path"], target)
        native = self.checkout / QUALIFIER.NATIVE
        native.mkdir(parents=True)
        before = next(source for source in self.manifest["sources"]
                      if source["role"] == "baseline_before")
        shutil.copyfile(self.checkout / before["path"], native / "unix_child.rs")
        (native / "mod.rs").write_text("#[cfg(unix)]\nmod unix_child;\n")
        self.cargo = '[package]\nname="diskgraph-engine"\nversion="0.1.0"\n'
        self.lock = ('version = 4\n\n[[package]]\nname = "cc"\nversion = "1.2.0"\n'
                     '\n[[package]]\nname = "diskgraph-engine"\nversion = "0.1.0"\n'
                     'dependencies = [\n "libc",\n]\n')
        (self.checkout / "crates/diskgraph-engine/Cargo.toml").write_text(self.cargo)
        (self.checkout / "Cargo.lock").write_text(self.lock)

    def test_active_restore_helper_must_match_exact_archive(self):
        active = self.checkout / "active/qualify-linux-atomic-launcher.py"
        active.parent.mkdir()
        shutil.copyfile(ROOT / "scripts/qualify-linux-atomic-launcher.py", active)
        (active.parent / "native_pid_namespace_restore.py").write_bytes(b"different active restore helper")
        with patch.object(QUALIFIER, "__file__", str(active)):
            with self.assertRaisesRegex(ValueError, "active.*restore.*digest mismatch"):
                QUALIFIER.validate_tooling(self.checkout, self.manifest, active)

    def test_changed_restore_helper_is_rejected_before_mount(self):
        helper = self.checkout / "scripts/native_pid_namespace_restore.py"
        helper.write_bytes(b"changed namespace restoration source")
        with self.assertRaisesRegex(ValueError, "digest mismatch.*native_pid_namespace_restore"):
            QUALIFIER.validate_tooling(self.checkout, self.manifest,
                                      ROOT / "scripts/qualify-linux-atomic-launcher.py")

    def test_frozen_material_mounts_exact_targets_without_scm_route(self):
        result = QUALIFIER.mount_candidate(self.checkout, self.manifest)
        for source in self.manifest["sources"]:
            if source.get("target"):
                self.assertEqual(QUALIFIER.digest(self.checkout / source["target"]), source["sha256"])
        self.assertEqual(len(result["declarations"]), 21)  # 20 模块和一个真实重导出。
        child = (self.checkout / self.manifest["baseline_unix_child"]["target"]).read_text()
        self.assertNotIn("spawn_scanner", child)
        self.assertNotIn("linux_scanner_launch", child)
        self.assertIn("UnixChildSetup::initialize", child)
        self.assertIn("retained", child)
        filter_path = self.checkout / (QUALIFIER.NATIVE + "linux_scanner_filter.rs")
        self.assertNotIn("fn install", filter_path.read_text())
        self.assertIn("fn program", filter_path.read_text())
        channel = (self.checkout / (QUALIFIER.NATIVE + "unix_control_channel.rs")).read_text()
        self.assertNotIn("fn pair", channel)
        self.assertIn("fn from_stream", channel)
        cargo = tomllib.loads((self.checkout / "crates/diskgraph-engine/Cargo.toml").read_text())
        self.assertEqual(cargo["build-dependencies"], {"cc": "1"})

    def test_eighteenth_exact_case_keeps_all_original_cases_and_inventory_gate(self):
        original = {
            'held_elf_replacement_preserves_image_and_closes_unrelated_descriptors',
            'production_stdin_fragments_and_real_eof_precede_original_pidfd_wait',
            'scanner_filter_rejects_process_clone_but_actual_pthread_still_runs',
            'main_pthread_exit_is_not_whole_thread_group_exit',
            'multithreaded_host_handlers_and_atfork_do_not_run_in_child_branch',
            'fatal_before_first_init_is_reaped_through_original_kernel_pidfd',
            'startup_channel_eof_causes_actual_natural_exit_and_original_reap',
            'after_birth_checkpoint_keeps_nonclone_primary_and_performs_actual_wait',
            'parent_observer_panic_reaps_before_original_payload_resumes',
            'original_pidfd_external_reap_does_not_target_other_live_child',
            'real_clone_denial_preserves_errno_and_has_no_fallback_or_birth_observer',
            'actual_close_range_error_is_typed_and_child_is_reaped_without_exec',
            'actual_getpgid_denial_preserves_original_errno_and_reaps',
            'actual_getsid_denial_preserves_original_errno_and_reaps',
            'startup_error_before_init_send_preserves_queued_errno_and_original_wait',
            'waitid_denied_after_physical_exit_returns_original_owner_for_actual_reap',
            'waitid_denied_unwind_keeps_original_box_and_transfers_unreaped_owner',
        }
        added = "exit_between_initial_waitid_and_poll_preserves_natural_exit_record"
        required = self.manifest["test_names"]
        self.assertEqual(self.manifest["expected_parent_cases"], 18)
        self.assertEqual(len(required), 18)
        self.assertEqual(set(required), original | {added})
        mounted = QUALIFIER.mount_candidate(self.checkout, self.manifest)
        declaration = '#[cfg(all(test, target_os = "linux"))]\nmod linux_atomic_launcher_ready_tests;\n'
        self.assertIn(declaration, mounted["declarations"])
        source = next(item for item in self.manifest["sources"]
                      if item.get("module") == "linux_atomic_launcher_ready_tests")
        self.assertEqual(QUALIFIER.digest(self.checkout / source["target"]), source["sha256"])
        inventory = "\n".join("native_child::qualified::" + name + ": test" for name in required)
        self.assertEqual(QUALIFIER.validate_inventory(inventory, required), required)
        for bad in (inventory.replace("native_child::qualified::" + added + ": test", ""),
                    inventory + "\n" + "native_child::qualified::" + added + ": test",
                    inventory.replace(added, "different_eighteenth_case")):
            with self.assertRaisesRegex(ValueError, "compiled inventory"):
                QUALIFIER.validate_inventory(bad, required)

    def test_corrupt_late_material_rejects_before_any_target_write(self):
        source = self.manifest["sources"][-1]
        (self.checkout / source["path"]).write_bytes(b"corrupt")
        child = self.checkout / self.manifest["baseline_unix_child"]["target"]
        before = child.read_bytes()
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            QUALIFIER.mount_candidate(self.checkout, self.manifest)
        self.assertEqual(child.read_bytes(), before)
        self.assertFalse((self.checkout / (QUALIFIER.NATIVE + "linux_atomic_launcher.rs")).exists())

    def test_path_traversal_and_repeated_target_are_not_admitted(self):
        for field, value in [("path", "../outside.rs"), ("target", QUALIFIER.NATIVE + "../outside.rs")]:
            manifest = copy.deepcopy(self.manifest)
            manifest["sources"][0][field] = value
            with self.assertRaises(ValueError):
                QUALIFIER.validate_sources(self.checkout, manifest)
        manifest = copy.deepcopy(self.manifest)
        manifest["sources"][1]["target"] = manifest["sources"][0]["target"]
        with self.assertRaisesRegex(ValueError, "repeated isolated target"):
            QUALIFIER.validate_sources(self.checkout, manifest)

    def test_only_existing_cc_reference_changes_and_repeat_is_idempotent(self):
        cargo, lock = QUALIFIER.add_cc_dependency(self.cargo, self.lock)
        expected = tomllib.loads(self.lock)
        expected["package"][1]["dependencies"] = ["cc", "libc"]
        self.assertEqual(tomllib.loads(lock), expected)
        self.assertEqual(QUALIFIER.add_cc_dependency(cargo, lock), (cargo, lock))
        with self.assertRaisesRegex(ValueError, "existing cc lock package"):
            QUALIFIER.add_cc_dependency(self.cargo, self.lock.replace('name = "cc"', 'name = "other"'))

    def test_shared_support_binding_is_actual_source_digest(self):
        _, path = QUALIFIER.shared_support()
        self.assertEqual(QUALIFIER.digest(path), self.manifest["shared_support_sha256"])

    def test_archived_tooling_drift_is_denied(self):
        QUALIFIER.validate_tooling(self.checkout, self.manifest, ROOT / "scripts/qualify-linux-atomic-launcher.py")
        source = self.checkout / self.manifest["required_outer_supervisor"]["source"]["path"]
        source.write_bytes(b"different archived supervisor")
        with self.assertRaisesRegex(ValueError, "archived tooling digest mismatch"):
            QUALIFIER.validate_tooling(self.checkout, self.manifest, ROOT / "scripts/qualify-linux-atomic-launcher.py")

    def test_running_qualifier_must_equal_exact_archive_and_manifest(self):
        changed = self.checkout / "different-active-qualifier.py"
        changed.write_bytes(b"different actual qualifier")
        with self.assertRaisesRegex(ValueError, "running qualifier differs"):
            QUALIFIER.validate_tooling(self.checkout, self.manifest, changed)

    def test_tooling_binding_cannot_substitute_another_path(self):
        manifest = copy.deepcopy(self.manifest)
        manifest["required_outer_supervisor"]["workflow"]["path"] = "../different.yml"
        with self.assertRaisesRegex(ValueError, "exact fixed source"):
            QUALIFIER.validate_tooling(self.checkout, manifest, ROOT / "scripts/qualify-linux-atomic-launcher.py")
        manifest = copy.deepcopy(self.manifest)
        manifest["shared_support_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "shared support binding disagrees"):
            QUALIFIER.validate_tooling(self.checkout, manifest, ROOT / "scripts/qualify-linux-atomic-launcher.py")


class StageBindingTests(unittest.TestCase):
    """固定六依赖的实际 active/archive 准入；只验证源码绑定，不执行命名空间。"""

    HELPERS = tuple("scripts/" + name for name in (
        "native_pid_namespace_run.py", "native_memfd_policy_plan.py",
        "native_memfd_policy_setup.py", "native_memfd_policy_stages.py", "native_memfd_prepared.py",
        "native_namespace_artifacts.py"))

    TESTS = tuple("scripts/tests/" + name for name in (
        "test_native_memfd_policy_stages.py", "test_native_memfd_prepared_contracts.py",
        "test_qualify_linux_memfd_stages.py", "test_native_namespace_artifacts.py"))

    def fixture(self, directory, profile):
        checkout, active = directory / "archive", directory / "active/scripts"
        active.mkdir(parents=True, exist_ok=True)
        atomic = json.loads((ROOT / QUALIFIER.MANIFEST).read_text())
        memfd_path = "docs/benchmarks/linux_memfd_execution_2026_10_05_candidate.json"
        memfd = json.loads((ROOT / memfd_path).read_text())
        for manifest in (atomic, memfd):
            manifest["policy_stage_tooling"] = [
                {"path": name, "sha256": QUALIFIER.digest(ROOT / name)} for name in self.HELPERS]
            manifest["policy_stage_tests"] = [
                {"path": name, "sha256": QUALIFIER.digest(ROOT / name)} for name in self.TESTS]

        def copy_bindings(value):
            if isinstance(value, dict):
                if "path" in value and "sha256" in value and (ROOT / value["path"]).is_file():
                    name = value["path"]
                    target = checkout / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / name, target)
                    value["sha256"] = QUALIFIER.digest(target)
                    if name.startswith("scripts/") and "/tests/" not in name:
                        shutil.copyfile(ROOT / name, active / Path(name).name)
                for child in value.values():
                    copy_bindings(child)
            elif isinstance(value, list):
                for child in value:
                    copy_bindings(child)

        copy_bindings(atomic)
        copy_bindings(memfd)
        atomic["qualifier_sha256"] = atomic["qualification_tooling"]["qualifier"]["sha256"]
        atomic["shared_support_sha256"] = atomic["qualification_tooling"]["shared_support"]["sha256"]
        path = checkout / QUALIFIER.MANIFEST
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(atomic))
        memfd["atomic_manifest_sha256"] = QUALIFIER.digest(path)
        spec = importlib.util.spec_from_file_location("fixed_memfd_binding", ROOT / "scripts/qualify-linux-memfd-execution.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return checkout, active, atomic if profile == "atomic" else memfd, module

    def validate(self, checkout, active, manifest, module, profile):
        with ExitStack() as stack:
            stack.enter_context(patch.object(QUALIFIER, "__file__", str(active / "qualify-linux-atomic-launcher.py")))
            stack.enter_context(patch.object(module, "ATOMIC", QUALIFIER))
            stack.enter_context(patch.object(module, "ATOMIC_PATH", active / "qualify-linux-atomic-launcher.py"))
            stack.enter_context(patch.object(module, "__file__", str(active / "qualify-linux-memfd-execution.py")))
            stack.enter_context(patch.dict(module.os.environ, {}, clear=True))
            if profile == "atomic":
                QUALIFIER.validate_tooling(checkout, manifest, active / "qualify-linux-atomic-launcher.py")
            else:
                module.validate_profile(checkout, manifest, active / "qualify-linux-memfd-execution.py")

    def test_each_fixed_helper_active_and_archive_drift_is_rejected_for_both_profiles(self):
        for profile in ("atomic", "memfd"):
            for name in self.HELPERS:
                for location in ("active", "archive"):
                    with self.subTest(profile=profile, helper=name, location=location), tempfile.TemporaryDirectory() as tmp:
                        checkout, active, manifest, module = self.fixture(Path(tmp), profile)
                        self.validate(checkout, active, manifest, module, profile)
                        target = active / Path(name).name if location == "active" else checkout / name
                        target.write_bytes(b"changed actual helper source")
                        with self.assertRaisesRegex(ValueError, "tooling.*digest mismatch"):
                            self.validate(checkout, active, manifest, module, profile)

    def test_missing_duplicate_and_substituted_fixed_bindings_are_rejected_for_both_profiles(self):
        for profile in ("atomic", "memfd"):
            for index in range(len(self.HELPERS)):
                for mutation in ("missing", "duplicate", "substituted"):
                    with self.subTest(profile=profile, index=index, mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                        checkout, active, manifest, module = self.fixture(Path(tmp), profile)
                        self.validate(checkout, active, manifest, module, profile)
                        bindings = manifest["policy_stage_tooling"]
                        if mutation == "missing":
                            del bindings[index]
                        elif mutation == "duplicate":
                            bindings[index] = copy.deepcopy(bindings[(index + 1) % len(bindings)])
                        else:
                            bindings[index]["path"] = "scripts/not_the_fixed_helper.py"
                        with self.assertRaisesRegex(ValueError, "fixed.*tooling bindings"):
                            self.validate(checkout, active, manifest, module, profile)


    def test_focused_tests_have_closed_fixed_archive_bindings_for_both_profiles(self):
        for profile in ("atomic", "memfd"):
            for index in range(len(self.TESTS)):
                for mutation in ("archive", "missing", "duplicate"):
                    with self.subTest(profile=profile, index=index, mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                        checkout, active, manifest, module = self.fixture(Path(tmp), profile)
                        self.validate(checkout, active, manifest, module, profile)
                        bindings = manifest["policy_stage_tests"]
                        if mutation == "archive":
                            (checkout / bindings[index]["path"]).write_bytes(b"changed focused test")
                        elif mutation == "missing":
                            del bindings[index]
                        else:
                            bindings[index] = copy.deepcopy(bindings[(index + 1) % len(bindings)])
                        with self.assertRaises(ValueError):
                            self.validate(checkout, active, manifest, module, profile)


class QualificationErrorTests(unittest.TestCase):
    """操作同一脚本实际 Python 分支，明确与 Linux 原生能力证明分开。"""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.output = Path(self.temporary.name)
        self.receipt = {"commit": "1" * 40}
        self.support = RecordedSupport()

    def run_command(self):
        return QUALIFIER.run_step(self.receipt, self.output, "case", ["qualified-command"],
                                  self.output, {}, self.support, timeout=1)

    def failing_open(self, error):
        original = Path.open

        def open_file(path, *args, **kwargs):
            stream = original(path, *args, **kwargs)
            return CloseFailure(stream, error) if args and args[0] == "wb" else stream

        return patch.object(Path, "open", open_file)

    def test_timeout_preserves_original_over_close_and_digest_errors(self):
        timeout = subprocess.TimeoutExpired(["qualified-command"], 1)
        close, digest_error = OSError("close sentinel"), OSError("digest sentinel")
        with self.failing_open(close), patch.object(QUALIFIER.subprocess, "Popen", return_value=ProcessResult(timeout)), \
                patch.object(QUALIFIER.os, "killpg"), patch.object(QUALIFIER, "digest", side_effect=digest_error):
            with self.assertRaises(subprocess.TimeoutExpired) as caught:
                self.run_command()
        self.assertIs(caught.exception, timeout)
        self.assertIs(self.receipt["case_log_close_error"], close)
        self.assertIs(self.receipt["case_log_digest_error"], digest_error)
        self.assertEqual(self.receipt["steps"][0]["status"], "failed")
        self.assertIn("outer PID namespace", self.receipt["steps"][0]["local_cleanup_scope"])

    def test_digest_without_primary_is_failure_not_passed(self):
        error = OSError("digest only sentinel")
        with patch.object(QUALIFIER.subprocess, "Popen", return_value=ProcessResult()), \
                patch.object(QUALIFIER, "digest", side_effect=error):
            with self.assertRaises(OSError) as caught:
                self.run_command()
        self.assertIs(caught.exception, error)
        self.assertEqual(self.receipt["steps"][0]["exit_code"], 0)
        self.assertEqual(self.receipt["steps"][0]["status"], "failed")

    def test_close_without_primary_is_failure_not_passed(self):
        error = OSError("close only sentinel")
        with self.failing_open(error), patch.object(QUALIFIER.subprocess, "Popen", return_value=ProcessResult()):
            with self.assertRaises(OSError) as caught:
                self.run_command()
        self.assertIs(caught.exception, error)
        self.assertIn("log_sha256", self.receipt["steps"][0])
        self.assertEqual(self.receipt["steps"][0]["status"], "failed")

    def test_log_stat_without_primary_is_failure_not_passed(self):
        error = OSError("stat only sentinel")
        with patch.object(QUALIFIER.subprocess, "Popen", return_value=ProcessResult()), \
                patch.object(Path, "stat", side_effect=error):
            with self.assertRaises(OSError) as caught:
                self.run_command()
        self.assertIs(caught.exception, error)
        self.assertEqual(self.receipt["steps"][0]["status"], "failed")

    def test_archive_preserves_tar_over_finish_close_and_digest_errors(self):
        tar = subprocess.CalledProcessError(2, ["tar"])
        cleanup, close, evidence = OSError("archive wait sentinel"), OSError("archive close sentinel"), OSError("archive digest sentinel")
        self.support.finish_archive = lambda process, receipt: (_ for _ in ()).throw(cleanup)
        with self.failing_open(close), patch.object(QUALIFIER.subprocess, "Popen", return_value=ProcessResult()), \
                patch.object(QUALIFIER.subprocess, "run", side_effect=tar), \
                patch.object(QUALIFIER, "digest", side_effect=evidence):
            with self.assertRaises(subprocess.CalledProcessError) as caught:
                QUALIFIER.extract_archive(self.output, self.output, self.output, self.receipt, {}, self.support)
        self.assertIs(caught.exception, tar)
        self.assertIs(self.receipt["archive_cleanup_error"], cleanup)
        self.assertIs(self.receipt["archive_log_close_error"], close)
        self.assertIs(self.receipt["archive_log_digest_error"], evidence)

    def test_archive_evidence_without_primary_propagates_failure(self):
        error = OSError("archive digest only sentinel")
        self.support.finish_archive = lambda process, receipt: process.wait(1)
        with patch.object(QUALIFIER.subprocess, "Popen", return_value=ProcessResult()), \
                patch.object(QUALIFIER.subprocess, "run"), patch.object(QUALIFIER, "digest", side_effect=error):
            with self.assertRaises(OSError) as caught:
                QUALIFIER.extract_archive(self.output, self.output, self.output, self.receipt, {}, self.support)
        self.assertIs(caught.exception, error)
        self.assertEqual(self.receipt["steps"][0]["status"], "failed")

    def test_archive_exit_error_is_not_replaced_by_log_close(self):
        close = OSError("archive close after exit sentinel")

        def finish(process, receipt):
            process.returncode = 3

        self.support.finish_archive = finish
        with self.failing_open(close), patch.object(QUALIFIER.subprocess, "Popen", return_value=ProcessResult()), \
                patch.object(QUALIFIER.subprocess, "run"):
            with self.assertRaises(subprocess.CalledProcessError) as caught:
                QUALIFIER.extract_archive(self.output, self.output, self.output, self.receipt, {}, self.support)
        self.assertEqual(caught.exception.returncode, 3)
        self.assertIs(self.receipt["archive_log_close_error"], close)
        self.assertEqual(self.receipt["steps"][0]["status"], "failed")


class NamespaceAdmissionTests(unittest.TestCase):
    """只测试编排否认规则；实际 PIDns 回收由外层原生资格单独证明。"""

    def setUp(self):
        self.environment = {"DG_NATIVE_NAMESPACE_SUPERVISED": "1",
                            "DG_NATIVE_PID_NAMESPACE": "pid:[111]",
                            "DG_NATIVE_NAMESPACE_OUTPUT": "/tmp/native-qualification"}
        self.output = Path("/tmp/native-qualification/runner/qualification").resolve()

    def namespace_links(self, namespace):
        original = QUALIFIER.os.readlink

        def readlink(path, *args, **kwargs):
            if path == "/proc/self":
                return "1"
            if path == "/proc/self/ns/pid":
                return namespace
            return original(path, *args, **kwargs)

        return patch.object(QUALIFIER.os, "readlink", readlink)

    def test_direct_execution_or_wrong_init_is_denied(self):
        with self.assertRaisesRegex(ValueError, "supervisor is required"):
            QUALIFIER.qualify_namespace({}, self.output)
        with patch.object(QUALIFIER.os, "getpid", return_value=2):
            with self.assertRaisesRegex(ValueError, "actual namespace init"):
                QUALIFIER.qualify_namespace(self.environment, self.output)

    def test_actual_namespace_or_output_mismatch_is_denied(self):
        with patch.object(QUALIFIER.os, "getpid", return_value=1), self.namespace_links("pid:[222]"):
            with self.assertRaisesRegex(ValueError, "does not match"):
                QUALIFIER.qualify_namespace(self.environment, self.output)
        with patch.object(QUALIFIER.os, "getpid", return_value=1), self.namespace_links("pid:[111]"):
            with self.assertRaisesRegex(ValueError, "supervised qualification directory"):
                QUALIFIER.qualify_namespace(self.environment, self.output / "different")

    def test_matching_observation_keeps_cleanup_explicitly_pending(self):
        with patch.object(QUALIFIER.os, "getpid", return_value=1), self.namespace_links("pid:[111]"):
            observed = QUALIFIER.qualify_namespace(self.environment, self.output)
        self.assertEqual(observed["cleanup"], "pending outer init wait receipt")


if __name__ == "__main__":
    unittest.main()
