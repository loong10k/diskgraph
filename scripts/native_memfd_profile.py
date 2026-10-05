"""仅固定 memfd policy 夹具的临时 AppArmor 资格；不授予生产扫描权限。

来源：Ubuntu 24.04 release notes 的 unconfined/userns profile 与 parser(8)。
规则只在当前外层 supervisor 持有期间加载；原 init 真正回收后才卸载。
"""
import hashlib
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import tempfile
import time


class NativeMemfdProfile:
    """唯一固定路径的 QA profile 生命周期，不接受任意规则或可执行文件。"""

    PARSER = "/usr/sbin/apparmor_parser"
    LABEL = "/proc/self/attr/current"
    PROFILES = "/sys/kernel/security/apparmor/profiles"

    def __init__(self, supervisor, deadline):
        self.supervisor = supervisor
        self.deadline = deadline
        self.fixture = supervisor.output / "qualification/fixtures/memfd-policy-namespace"
        self.name = "diskgraph_memfd_policy_" + secrets.token_hex(16)
        self.definition = ("abi <abi/4.0>,\ninclude <tunables/global>\n"
                           f'profile {self.name} "{self.fixture}" flags=(unconfined) {{\n  userns,\n}}\n').encode()
        self.add_attempted = False
        self.record = {"scope": "QA namespace preparation, including inheriting fixture descendants; not production permission",
                       "name": self.name, "fixture": str(self.fixture),
                       "definition": self.definition.decode(),
                       "definition_sha256": hashlib.sha256(self.definition).hexdigest(),
                       "helper_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                       "loaded_verified": False, "unloaded_verified": False, "pending": False, "steps": []}
        supervisor.receipt["memfd_profile"] = self.record

    def _read(self, path, limit):
        stream = Path(path).open("rb")
        primary = None
        try:
            value = stream.read(limit + 1)
            if len(value) > limit:
                raise OverflowError(f"bounded AppArmor witness exceeded {limit} bytes: {path}")
            return value
        except BaseException as error:
            primary = error
            raise
        finally:
            try:
                stream.close()
            except BaseException as error:
                if primary is None:
                    raise
                self.supervisor.failure("memfd_profile_read_primary", primary)
                self.supervisor.failure("memfd_profile_read_close", error)

    def require_runner_label(self):
        """参数：无；返回：实际 unconfined 标签，拒绝在既有受限域中扩大权限。"""
        label = self._read(self.LABEL, 256).decode("utf-8").strip()
        if label != "unconfined":
            raise PermissionError("temporary QA profile requires actual unconfined outer runner")
        return label

    def _check_paths(self):
        raw = str(self.fixture)
        if not re.fullmatch(r"/[A-Za-z0-9_./-]+", raw) or len(raw.encode()) > 4096:
            raise ValueError("fixed profile path must exclude AARE, quotes, whitespace and control bytes")
        if self.supervisor.output != self.supervisor.output.resolve(strict=True):
            raise ValueError("profile output path must already be canonical, without symlink aliases")
        for path in [self.fixture, *self.fixture.parents]:
            try:
                mode = os.lstat(path).st_mode
            except FileNotFoundError:
                continue
            if stat.S_ISLNK(mode) or (path != self.fixture and not stat.S_ISDIR(mode)):
                raise ValueError("profile attachment path contains a symlink or non-directory ancestor")
            if path == self.fixture and not stat.S_ISREG(mode):
                raise ValueError("profile fixture path is not a regular file")
        parser = os.lstat(self.PARSER)
        if (not stat.S_ISREG(parser.st_mode) or parser.st_uid != 0 or
                parser.st_mode & (0o022 | stat.S_ISUID | stat.S_ISGID)):
            raise PermissionError("fixed AppArmor parser must be root-owned regular non-writable non-set-id")

    def _present(self):
        raw = self._read(self.PROFILES, 1 << 20)
        matches = [line for line in raw.decode("utf-8").splitlines()
                   if line == self.name or line.startswith(self.name + " ")]
        if len(matches) > 1:
            raise ValueError("duplicate exact AppArmor profile witness")
        if not matches:
            return None
        if matches[0] not in (self.name + " (unconfined)", self.name + " (enforce)"):
            raise ValueError("loaded profile has an unexpected mode or stacked identity")
        return matches[0]

    def _step(self, phase, operation, *, cleanup=False):
        # parser(8) 的 jobs=0 明确在主进程编译；不继承宿主 parser.conf 的自由选项。
        command = [self.PARSER, operation, "--jobs=0", "--config-file=/dev/null",
                   "--skip-cache", "--warn=rule-not-enforced", "--Werror=rule-not-enforced"]
        remaining = 10.0 if cleanup else min(10.0, self.deadline - time.monotonic())
        if remaining <= 0:
            raise TimeoutError("original namespace qualification deadline elapsed before profile preparation")
        record = {"phase": phase, "command": command, "definition_sha256": self.record["definition_sha256"],
                  "cleanup_only_window": cleanup}
        self.record["steps"].append(record)
        streams, secondary, primary = {}, [], None
        started = time.monotonic()
        try:
            for name in ("stdout", "stderr"):
                streams[name] = tempfile.TemporaryFile()
            result = subprocess.run(command, input=self.definition, stdout=streams["stdout"],
                                    stderr=streams["stderr"], timeout=remaining, start_new_session=True,
                                    env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C", "LC_ALL": "C"})
            record["returncode"] = result.returncode
            if result.returncode != 0:
                primary = subprocess.CalledProcessError(result.returncode, command)
        except BaseException as error:
            primary = error
        finally:
            for name, stream in streams.items():
                try:
                    stream.seek(0)
                    raw = stream.read((64 << 10) + 1)
                    record[name + "_bytes"] = len(raw)
                    record[name + "_sha256"] = hashlib.sha256(raw).hexdigest()
                    record[name + "_diagnostic"] = raw[:2048].decode("utf-8", errors="replace")
                    if len(raw) > (64 << 10):
                        raise OverflowError("AppArmor parser output exceeded independent 64 KiB cap")
                except BaseException as error:
                    if primary is None:
                        primary = error
                    else:
                        secondary.append(("memfd_profile_parser_capture", error))
                try:
                    stream.close()
                except BaseException as error:
                    if primary is None:
                        primary = error
                    else:
                        secondary.append(("memfd_profile_parser_close", error))
            record["elapsed_seconds"] = time.monotonic() - started
            if primary is not None:
                if secondary:
                    self.supervisor.failure("memfd_profile_parser_primary", primary)
                    for phase, error in secondary:
                        self.supervisor.failure(phase, error)
                raise primary

    def load(self):
        """参数：无；返回：仅在固定规则真实加载与精确内核标签见证后正常返回。"""
        self.record["outer_label"] = self.require_runner_label()
        self._check_paths()
        if self._read("/sys/module/apparmor/parameters/enabled", 16).strip() != b"Y":
            raise RuntimeError("actual AppArmor is not enabled; missing qualification is not a skip")
        if self._present() is not None:
            raise FileExistsError("unique QA profile name already exists; no profile replacement permitted")
        self._step("parse", "--skip-kernel-load")
        # 先保存清理责任，再尝试 add；成功加载但后续读取失败也必须尝试回收。
        self.add_attempted = True
        self.record["pending"] = True
        self._step("load", "--add")
        witness = self._present()
        if witness is None:
            raise RuntimeError("AppArmor parser succeeded without the exact loaded profile")
        self.record.update(loaded_verified=True, loaded_witness=witness)

    def environment(self):
        """参数：无；返回：固定 profile/fixture 见证，不允许客户端指定可执行文件。"""
        if not self.record["loaded_verified"]:
            raise RuntimeError("unverified profile cannot supply namespace preparation markers")
        return {"DG_MEMFD_QA_PROFILE": self.name, "DG_MEMFD_QA_FIXTURE": str(self.fixture)}

    def finish(self):
        """参数：无；返回：原 init/capture 回收后卸载，失败保留原主错并标记未清理。"""
        if not self.add_attempted:
            return
        try:
            if ((self.supervisor.pid is not None and not self.supervisor.reaped) or
                    self.supervisor.streams or any(not item[0].closed for item in self.supervisor.logs.values())):
                raise RuntimeError("profile cannot be removed before actual init reap and capture closure")
            try:
                present = self._present()
            except BaseException as error:
                # 已尝试 add 的清理责任不能因为诊断读取损坏而丢失；仍移除唯一原定义。
                self.supervisor.failure("memfd_profile_remove_witness", error)
                present = "unavailable"
            if present is None and not self.record["loaded_verified"]:
                self.record.update(unloaded_verified=True, pending=False)
                return
            if present is None:
                raise RuntimeError("loaded profile disappeared before owned cleanup")
            self._step("remove", "--remove", cleanup=True)
            if self._present() is not None:
                raise RuntimeError("profile removal returned without exact kernel disappearance")
            self.record.update(unloaded_verified=True, pending=False)
        except BaseException as error:
            self.supervisor.failure("memfd_profile_remove", error)
