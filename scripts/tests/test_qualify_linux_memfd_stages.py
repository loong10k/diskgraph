#!/usr/bin/env python3
"""现有 qualifier.main 的普通准备阶段合同；模拟 Cargo 日志，不是原生验收。

原六案名称来自现实际清单；不导入计划中 helper，不用缺 API/ImportError 作 RED。
"""
from contextlib import ExitStack, contextmanager
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/qualify-linux-memfd-execution.py"
SPEC = importlib.util.spec_from_file_location("memfd_fixed_prepare_stages", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class MemfdPrepareStageContracts(unittest.TestCase):
    """只替换准备环境和外部命令；受测 main 的实际阶段选择与 receipt 不替代。"""

    def invoke(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "qualification"
            manifest = json.loads((ROOT / MODULE.MANIFEST).read_text())
            groups = manifest["test_groups"]
            state = SimpleNamespace(native=[], checkout=None, receipt=None, remaining_material=False)

            @contextmanager
            def isolated(output, receipt):
                checkout = Path(tempfile.mkdtemp(prefix="checkout-", dir=output))
                state.checkout = checkout
                try:
                    yield checkout
                finally:
                    shutil.rmtree(checkout)

            support = SimpleNamespace(isolated_checkout=isolated,
                                      secondary=lambda receipt, phase, error: receipt.update({phase: repr(error)}))

            def archive(_repo, checkout, *_):
                state.checkout = checkout
                # main 会对 archived manifest 本体哈希；不能仅替代 JSON 返回而缺原文件。
                archived_manifest = checkout / MODULE.MANIFEST
                archived_manifest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / MODULE.MANIFEST, archived_manifest)
                for name in ("crates/diskgraph-engine/tests/fixtures/linux_atomic_launcher_fixture.c",
                             MODULE.NATIVE + "linux_atomic_spawn.c", manifest["policy_fixture"]):
                    path = checkout / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b"explicit assembly source, not compiled native proof")

            def step(receipt, out, name, command, cwd, environment, *_):
                path = out / (name + ".log")
                if name == "captured-commit":
                    path.write_text("a" * 40 + "\n")
                elif name.startswith("fixture-"):
                    Path(command[-1]).write_bytes(b"modeled fixture artifact, not executable")
                    path.write_text("modeled compiler exit\n")
                elif name.startswith("inventory-"):
                    group = next(group for group in groups if name == "inventory-" + group["filter"])
                    path.write_text("\n".join(test + ": test" for test in group["names"]) + "\n")
                elif name.startswith("native-"):
                    group = next(group for group in groups if name == "native-" + group["filter"])
                    state.native.append(group["filter"])
                    path.write_text("test result: ok. " + str(len(group["names"])) +
                                    " passed; 0 failed; 0 ignored;\n")
                elif name == "prepared-libtest":
                    # 新真实 no-run 的明确装配模型；文件只供摘要/路径验证，不执行。
                    binary = cwd / "target-memfd-qualification/debug/deps/diskgraph_engine-0123456789abcdef"
                    binary.parent.mkdir(parents=True, exist_ok=True)
                    binary.write_bytes(b"explicit modeled original libtest, not native executable proof")
                    binary.chmod(0o755)
                    path.write_text(json.dumps({"reason": "compiler-artifact", "profile": {"test": True},
                                                "target": {"name": "diskgraph_engine", "kind": ["lib"]},
                                                "executable": str(binary)}) + "\n")
                else:
                    raise AssertionError("unexpected free command in existing preparation route: " + name)
                return path

            with ExitStack() as stack:
                stack.enter_context(patch.object(MODULE.sys, "argv", [str(SOURCE), "--output-dir", str(output)]))
                stack.enter_context(patch.object(MODULE.sys, "platform", "linux"))
                stack.enter_context(patch.object(MODULE.os, "uname", return_value=SimpleNamespace(machine="x86_64")))
                stack.enter_context(patch.dict(os.environ, {"DG_MEMFD_QA_PHASE": "prepare",
                                                          "RUSTFLAGS": "-Dwarnings"}, clear=True))
                stack.enter_context(patch.object(MODULE.ATOMIC, "qualify_namespace", return_value=
                                                {"scope": "modeled PID1 qualification, not native evidence"}))
                stack.enter_context(patch.object(MODULE.ATOMIC, "shared_support", return_value=(support, SOURCE)))
                stack.enter_context(patch.object(MODULE.ATOMIC, "run_step", side_effect=step))
                stack.enter_context(patch.object(MODULE.ATOMIC, "extract_archive", side_effect=archive))
                stack.enter_context(patch.object(MODULE, "validate_profile", return_value=
                                                {"sources": [], "shared_support_sha256": MODULE.ATOMIC.digest(SOURCE)}))
                stack.enter_context(patch.object(MODULE, "mount_profile", return_value={"scope": "modeled mount"}))
                stack.enter_context(patch.object(MODULE.ATOMIC, "object_evidence"))
                MODULE.main()
            state.receipt = json.loads((output / "receipt.json").read_text())
            state.remaining_material = state.checkout is not None and state.checkout.is_dir()
            return state

    def test_prepare_runs_execution_two_and_denial_one_without_policy_cargo(self):
        state = self.invoke()
        self.assertEqual(state.native, ["linux_scan_image_execution_tests", "linux_memfd_denial_tests"],
                         "ordinary runner stage must not invoke the old unprivileged policy C path")
        self.assertEqual(sum(item["passed"] for item in state.receipt["native_results"]), 3)

    def test_prepared_three_cannot_claim_six_or_destroy_pending_case_material(self):
        state = self.invoke()
        self.assertEqual(state.receipt["status"], "prepared_policy_cases_pending",
                         "ordinary stage cannot declare all six parent cases completed")
        self.assertEqual(state.receipt["executed_parent_cases"], 3)
        self.assertTrue(state.remaining_material, "compiled source/image materials must survive until all case waits")


if __name__ == "__main__":
    unittest.main()
