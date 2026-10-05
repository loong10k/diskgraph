#!/usr/bin/env python3
"""生成/核验同版本 helper 的安装材料；清单不是执行镜像或安装信任证明。"""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import tomllib


ROOT = Path(__file__).resolve().parent.parent
PIN = "158f9cc2f0b332194a3ffc5acec47760c99146d8"
MANIFEST_NAME = "scan-worker-manifest.json"
MAX_MANIFEST_BYTES = 16 * 1024
MAX_IMAGE_BYTES = 128 * 1024 * 1024
CHUNK_BYTES = 64 * 1024
FIELDS = {"schema_version", "package_version", "target", "protocol_version",
          "pinned_scanner_revision", "scanner_source_manifest_sha256", "executable"}


def workspace_version():
    """读取当前构建的真实 workspace 版本，不从运行时请求推导。"""
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def source_digest():
    """引用不可修改的上游源摘要；不以此替代 vendor 源码一致性验收。"""
    path = ROOT / "crates/diskgraph-disktree-core/UPSTREAM_DIGESTS.txt"
    with admitted_file(path, MAX_MANIFEST_BYTES) as (stream, before):
        data = stream.read(before.st_size)
        if len(data) != before.st_size:
            raise ValueError("truncated pinned scanner source manifest")
    if PIN.encode("ascii") not in data:
        raise ValueError("invalid pinned scanner source manifest")
    return hashlib.sha256(data).hexdigest()


def executable_name(target):
    """仅允许有限的 target 文本与固定 helper 名，不接受客户端路径。"""
    if not isinstance(target, str) or len(target) > 128 or not re.fullmatch(r"[a-z0-9_]+(?:-[a-z0-9_]+){2,}", target):
        raise ValueError("invalid build target")
    return "diskgraph-scan-worker" + (".exe" if "-windows-" in target else "")


def file_identity(entry):
    """安装材料的变更见证，不作为平台原生执行身份能力。"""
    return (entry.st_dev, entry.st_ino, entry.st_size, entry.st_mtime_ns, entry.st_ctime_ns)


@contextmanager
def admitted_file(path, byte_limit):
    """非阻塞打开后验证同一普通文件；拒绝准入竞态及读取期间的变更。"""
    path = Path(path)
    before_path = path.lstat()
    if not stat.S_ISREG(before_path.st_mode):
        raise ValueError(f"artifact is not a regular non-link file: {path.name}")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    flags |= getattr(os, "O_NONBLOCK", 0)
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, "rb", buffering=0) as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or file_identity(before_path) != file_identity(before):
            raise ValueError("artifact changed before file admission")
        if not 0 < before.st_size <= byte_limit:
            raise ValueError("artifact byte budget exceeded or empty")
        yield stream, before
        if stream.read(1) or file_identity(os.fstat(stream.fileno())) != file_identity(before):
            raise ValueError("artifact changed during bounded read")
        after_path = path.lstat()
        if not stat.S_ISREG(after_path.st_mode) or file_identity(after_path) != file_identity(before):
            raise ValueError("artifact namespace changed during read")


def image_metadata(path):
    """按同一打开文件的剩余长度窄读；变化、链接或未完整读取时不提供摘要。"""
    path = Path(path)
    with admitted_file(path, MAX_IMAGE_BYTES) as (stream, before):
        remaining = before.st_size
        digest = hashlib.sha256()
        while remaining:
            chunk = stream.read(min(CHUNK_BYTES, remaining))
            if not chunk:
                raise ValueError("helper changed during bounded digest")
            digest.update(chunk)
            remaining -= len(chunk)
        result = {"name": path.name, "bytes": before.st_size, "sha256": digest.hexdigest()}
    return result


def copy_artifact(source, destination):
    """从已准入的同一文件句柄有界复制，目标必须是全新 staging 文件。"""
    with admitted_file(source, MAX_IMAGE_BYTES) as (stream, before):
        remaining = before.st_size
        with Path(destination).open("xb") as output:
            while remaining:
                chunk = stream.read(min(CHUNK_BYTES, remaining))
                if not chunk:
                    raise ValueError("artifact changed during bounded copy")
                output.write(chunk)
                remaining -= len(chunk)
        os.chmod(destination, stat.S_IMODE(before.st_mode))


def bounded_digest(path, byte_limit):
    """独立限制归档或二进制读取量；不把摘要材料称为执行信任。"""
    with admitted_file(path, byte_limit) as (stream, before):
        remaining = before.st_size
        digest = hashlib.sha256()
        while remaining:
            chunk = stream.read(min(CHUNK_BYTES, remaining))
            if not chunk:
                raise ValueError("artifact changed during bounded digest")
            digest.update(chunk)
            remaining -= len(chunk)
        result = digest.hexdigest()
    return result


def create_manifest(bin_dir, target, version):
    """从真实构建 artifact 创建闭合材料；不授予运行时执行或发布许可。"""
    if not isinstance(version, str) or len(version) > 128 or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.+-]+)?", version):
        raise ValueError("invalid package version")
    return {
        "schema_version": 1, "package_version": version, "target": target,
        "protocol_version": 2, "pinned_scanner_revision": PIN,
        "scanner_source_manifest_sha256": source_digest(),
        "executable": image_metadata(Path(bin_dir) / executable_name(target)),
    }


def verify_manifest(bin_dir, manifest, target, version):
    """与受信调用方预期的构建配置比较；相邻清单与文件不能互相证明安装信任。"""
    if type(manifest) is not dict or set(manifest) != FIELDS:
        raise ValueError("unknown or missing worker manifest fields")
    image = manifest["executable"]
    if type(image) is not dict or set(image) != {"name", "bytes", "sha256"}:
        raise ValueError("unknown or missing executable manifest fields")
    if type(manifest["schema_version"]) is not int or type(manifest["protocol_version"]) is not int or type(image["bytes"]) is not int:
        raise ValueError("invalid worker manifest numeric fields")
    expected = create_manifest(bin_dir, target, version)
    if manifest != expected:
        raise ValueError("worker manifest differs from the expected build artifact")


def unique_object(pairs):
    """重复 JSON key 不得悄悄覆盖已有清单字段。"""
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate worker manifest field")
        result[key] = value
    return result


def read_manifest(path):
    """先限制正文再解码；拒绝清单链接、超限与重复字段。"""
    with admitted_file(path, MAX_MANIFEST_BYTES) as (stream, before):
        remaining = before.st_size
        data = bytearray()
        while remaining:
            chunk = stream.read(min(CHUNK_BYTES, remaining))
            if not chunk:
                raise ValueError("manifest changed during bounded read")
            data.extend(chunk)
            remaining -= len(chunk)
    return json.loads(data, object_pairs_hook=unique_object)


def write_manifest(bin_dir, target, version):
    """仅在构建目录原子写入完整安装材料；重建可以替换旧清单。"""
    bin_dir = Path(bin_dir)
    manifest = create_manifest(bin_dir, target, version)
    payload = (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode("utf-8")
    if len(payload) > MAX_MANIFEST_BYTES:
        raise ValueError("worker manifest byte budget exceeded")
    descriptor, temporary = tempfile.mkstemp(prefix=".scan-worker-manifest-", dir=bin_dir)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, 0o644)
        os.replace(temporary, bin_dir / MANIFEST_NAME)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", default=workspace_version())
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    if args.verify:
        manifest = read_manifest(args.bin_dir / MANIFEST_NAME)
        verify_manifest(args.bin_dir, manifest, args.target, args.version)
    else:
        manifest = write_manifest(args.bin_dir, args.target, args.version)
    print(json.dumps(manifest, sort_keys=True))


if __name__ == "__main__":
    main()
