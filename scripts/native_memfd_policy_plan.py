"""固定三项策略案与普通 runner 准备材料；不提供 root argv/sysctl 通用接口。"""
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import stat


ARTIFACTS = runpy.run_path(str(Path(__file__).with_name("native_namespace_artifacts.py")))["NativeNamespaceArtifacts"]


class NativeMemfdPolicyPlan:
    """来源：physical-scan-process 固定两阶段 QA；名称必须与原 Rust inventory 同一组。"""

    NAMES = (
        "native_child::linux_memfd_policy_tests::policy_zero_executes_sealed_a_after_path_replacement",
        "native_child::linux_memfd_policy_tests::policy_one_executes_sealed_a_after_path_replacement",
        "native_child::linux_memfd_policy_tests::policy_two_preserves_native_execution_creation_denial",
    )
    FILE = "prepared-policy.json"

    @staticmethod
    def digest(path):
        digest = hashlib.sha256()
        for block in NativeMemfdPolicyPlan.blocks(path, 128 << 20):
            digest.update(block)
        return digest.hexdigest()

    @staticmethod
    def blocks(path, limit):
        """参数：固定材料与字节界；返回：实际普通 FD 的块，FIFO/symlink 竞态不得阻塞 root。"""
        fd = ARTIFACTS.open_read(path)
        primary = None
        try:
            metadata = os.fstat(fd)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > limit:
                raise ValueError("prepared material is not a bounded actual regular descriptor")
            total = 0
            while block := os.read(fd, 64 << 10):
                total += len(block)
                if total > limit:
                    raise ValueError("prepared material grew beyond its admitted bound")
                yield block
        except BaseException as error:
            primary = error
            raise
        finally:
            try:
                os.close(fd)
            except BaseException as error:
                if primary is None:
                    raise
                primary.add_note(f"prepared descriptor close secondary: {type(error).__name__} errno={getattr(error, 'errno', None)}")

    @staticmethod
    def path(output, relative):
        """参数：固定输出目录与受检相对名；返回：同目录实际普通文件，不接受 alias。"""
        if not isinstance(relative, str) or not re.fullmatch(r"[A-Za-z0-9_./-]{1,512}", relative):
            raise ValueError("invalid fixed prepared material path")
        raw = Path(relative)
        if raw.is_absolute() or ".." in raw.parts:
            raise ValueError("prepared material escapes output")
        path = output / raw
        if path != path.resolve(strict=True) or not stat.S_ISREG(path.stat().st_mode):
            raise ValueError("prepared material must be canonical regular source")
        return path

    @classmethod
    def record(cls, output, binary, inventory):
        """参数：普通 runner 构建的原 libtest；返回：只含固定相对名/摘要的准备记录。"""
        if inventory[-3:] != list(cls.NAMES) or len(inventory) != 6:
            raise ValueError("prepared inventory must preserve original six names")
        relative = binary.relative_to(output).as_posix()
        if not re.fullmatch(r"prepared-checkout/target-memfd-qualification/debug/deps/diskgraph_engine-[0-9a-f]+", relative):
            raise ValueError("unexpected original libtest artifact path")
        binary = cls.path(output, relative)
        if not os.access(binary, os.X_OK) or binary.stat().st_size > (128 << 20):
            raise ValueError("bounded executable original libtest required")
        fixtures = []
        for number in (1, 2):
            name = f"fixtures/image-{number}"
            path = cls.path(output, name)
            fixtures.append({"path": name, "sha256": cls.digest(path), "bytes": path.stat().st_size})
        return {"schema_version": 1, "checkout": "prepared-checkout", "inventory": inventory,
                "binary": {"path": relative, "sha256": cls.digest(binary), "bytes": binary.stat().st_size},
                "fixtures": fixtures}

    @classmethod
    def read(cls, output):
        """参数：固定输出目录；返回：有界、闭合记录，不授予特权执行材料许可。"""
        source = cls.path(output, cls.FILE)
        if source.stat().st_size > (64 << 10):
            raise ValueError("prepared policy record exceeds 64 KiB")
        value = json.loads(b"".join(cls.blocks(source, 64 << 10)))
        if (not isinstance(value, dict) or set(value) != {"schema_version", "checkout", "inventory", "binary", "fixtures"}
                or value["schema_version"] != 1 or value["checkout"] != "prepared-checkout"
                or not isinstance(value["inventory"], list) or not isinstance(value["fixtures"], list)
                or len(value["fixtures"]) != 2
                or len(value["inventory"]) != 6 or value["inventory"][-3:] != list(cls.NAMES)):
            raise ValueError("invalid closed fixed policy preparation")
        for index, item in enumerate([value["binary"], *value["fixtures"]]):
            if (not isinstance(item, dict) or set(item) != {"path", "sha256", "bytes"}
                    or not isinstance(item["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"])
                    or type(item["bytes"]) is not int or not 0 < item["bytes"] <= (128 << 20)):
                raise ValueError("invalid bounded prepared artifact")
            expected = (r"prepared-checkout/target-memfd-qualification/debug/deps/diskgraph_engine-[0-9a-f]+"
                        if index == 0 else rf"fixtures/image-{index}")
            if not re.fullmatch(expected, item["path"]):
                raise ValueError("artifact does not identify the fixed original build")
        return value

    @classmethod
    def verify_runner_material(cls, output):
        """仅普通 runner 调用：重新实读准备产物，root 不加载或执行这些文件。"""
        if os.geteuid() == 0:
            raise PermissionError("prepared runner code must never execute as root")
        value = cls.read(output)
        for item in [value["binary"], *value["fixtures"]]:
            path = cls.path(output, item["path"])
            if path.stat().st_size != item["bytes"] or cls.digest(path) != item["sha256"]:
                raise ValueError("prepared artifact changed before exact case execution")
        return value
