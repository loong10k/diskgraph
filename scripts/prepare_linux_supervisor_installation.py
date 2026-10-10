#!/usr/bin/env python3
"""准备隔离 Linux 监督安装树；不启动服务，不认证请求，不替代监督生命周期验收。"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import stat
import sys
import tarfile

MAX_IMAGE_BYTES = 64 << 20
MAX_ARCHIVE_BYTES = 256 << 20
NAMES = ('diskgraph', 'diskgraph-mcp', 'diskgraph-scan-worker')
PIN = '158f9cc2f0b332194a3ffc5acec47760c99146d8'


def validate_identities(service_uid, service_gid, frontend_uids):
    """校验独立安装者指定的专用服务身份，返回不引用调用者列表的策略。"""
    values = [service_uid, service_gid, *frontend_uids]
    if any(type(value) is not int or not 0 < value < (1 << 32) - 1 for value in values):
        raise ValueError('invalid or privileged numeric identity')
    if not 1 <= len(frontend_uids) <= 64 or len(set(frontend_uids)) != len(frontend_uids):
        raise ValueError('frontend identity set must be nonempty, unique and bounded')
    if service_uid in frontend_uids:
        raise ValueError('dedicated service identity must be excluded from every frontend')
    return {'service_uid': service_uid, 'service_gid': service_gid,
            'frontend_uids': list(frontend_uids)}


def verify_root_directory(fd):
    """以打开对象验证 root 目录；不修复已存在目录的所有权或权限。"""
    info = os.fstat(fd)
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022:
        raise ValueError('installation ancestry must be root-owned and not group/other writable')


def create_fresh_prefix(prefix):
    """逐组件禁止链接并保留原父句柄；只独占创建最终安装目录，拒绝覆盖旧安装。"""
    prefix = Path(prefix)
    if not prefix.is_absolute() or '..' in prefix.parts or len(os.fsencode(prefix)) > 1024:
        raise ValueError('invalid absolute installation prefix')
    if len(prefix.parts) < 3:
        raise ValueError('installation prefix must have a protected parent')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        verify_root_directory(fd)
        for component in prefix.parts[1:-1]:
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                            dir_fd=fd)
            try:
                verify_root_directory(child)
            except BaseException:
                os.close(child)
                raise
            os.close(fd)
            fd = child
        os.mkdir(prefix.name, mode=0o755, dir_fd=fd)
        child = os.open(prefix.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                        dir_fd=fd)
        try:
            verify_root_directory(child)
            os.fchmod(child, 0o755)
            os.fsync(child)
            os.fsync(fd)
            return child
        except BaseException:
            os.close(child)
            raise
    finally:
        os.close(fd)


def write_at(directory, name, stream, limit, mode, owner=None, validate_header=None):
    """独占写入 held 目录下单个固定名称，逐次扣实际字节，同步后设置所需权限。"""
    if '/' in name or name in ('.', '..'):
        raise ValueError('invalid fixed entry name')
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                 0o600, dir_fd=directory)
    size, digest = 0, hashlib.sha256()
    try:
        with os.fdopen(fd, 'wb', closefd=False) as output:
            while True:
                block = stream.read(min(65536, limit - size + 1))
                if not block:
                    break
                if size == 0 and validate_header is not None:
                    validate_header(block)
                size += len(block)
                if size > limit:
                    raise ValueError('installation entry exceeds byte limit')
                digest.update(block)
                output.write(block)
            output.flush()
        if owner is not None:
            os.fchown(fd, *owner)
        os.fchmod(fd, mode)
        os.fsync(fd)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size != size:
            raise ValueError('exclusive installation entry identity changed')
        if owner is not None and (info.st_uid, info.st_gid) != owner:
            raise ValueError('service slot owner did not match the requested identity')
        return {'bytes': size, 'sha256': digest.hexdigest(), 'device': info.st_dev,
                'inode': info.st_ino, 'uid': info.st_uid, 'gid': info.st_gid,
                'mode': stat.S_IMODE(info.st_mode), 'links': info.st_nlink}
    finally:
        os.close(fd)


class BoundedArchiveInput:
    """累计约束真实解压输入，不以压缩文件长度代替展开成本。"""
    def __init__(self, stream):
        self.stream, self.remaining = stream, MAX_ARCHIVE_BYTES

    def read(self, size=65536):
        block = self.stream.read(min(max(size, 1), 65536, self.remaining + 1))
        self.remaining -= len(block)
        if self.remaining < 0:
            raise ValueError('expanded archive exceeds byte limit')
        return block


def validate_elf_header(block, target):
    """检查首块的64位小端ELF与原生机器标识；不代替签发来源或动态加载验收。"""
    machine = {'aarch64-unknown-linux-gnu': 183, 'x86_64-unknown-linux-gnu': 62}.get(target)
    if machine is None or len(block) < 64 or block[:7] != b'\x7fELF\x02\x01\x01':
        raise ValueError('role image is not a native ELF64 executable')
    if int.from_bytes(block[16:18], 'little') not in (2, 3) or int.from_bytes(block[18:20], 'little') != machine or int.from_bytes(block[20:24], 'little') != 1:
        raise ValueError('ELF image type, machine or version mismatch')


def install_images(archive, images, target):
    """从已核验 root 独占快照流式安装三个固定镜像；不调用 extractall 或执行镜像。"""
    result, manifest, seen = {}, None, set()
    expected_root = 'diskgraph-' + target
    with gzip.GzipFile(fileobj=archive) as expanded:
        with tarfile.open(fileobj=BoundedArchiveInput(expanded), mode='r|', bufsize=65536) as package:
            for member in package:
                if len(seen) >= 128 or member.name in seen:
                    raise ValueError('duplicate or excessive archive entries')
                seen.add(member.name)
                path = PurePosixPath(member.name)
                if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != expected_root:
                    raise ValueError('archive member escapes fixed package root')
                if len(member.name.encode('utf-8')) > 2048 or not (member.isdir() or member.isfile()):
                    raise ValueError('links or nonordinary package entries are refused')
                if not member.isfile():
                    continue
                if member.size < 0 or member.size > MAX_IMAGE_BYTES:
                    raise ValueError('archive member exceeds entry limit')
                if len(path.parts) != 3 or path.parts[1] != 'bin':
                    continue
                name = path.parts[2]
                if name == 'scan-worker-manifest.json':
                    if member.size > 16384:
                        raise ValueError('oversized worker manifest')
                    manifest = json.loads(package.extractfile(member).read(16385))
                elif name in NAMES:
                    result[name] = write_at(images, name, package.extractfile(member), MAX_IMAGE_BYTES, 0o555,
                                            validate_header=lambda block: validate_elf_header(block, target))
                    if result[name]['bytes'] != member.size or not member.size:
                        raise ValueError('incomplete or empty role image')
                else:
                    raise ValueError('unknown executable entry')
    if set(result) != set(NAMES) or not isinstance(manifest, dict):
        raise ValueError('incomplete package image set')
    if manifest.get('schema_version') != 1 or manifest.get('target') != target or manifest.get('protocol_version') != 2 or manifest.get('pinned_scanner_revision') != PIN:
        raise ValueError('worker platform, protocol or upstream pin mismatch')
    expected = manifest.get('executable', {})
    if expected.get('name') != 'diskgraph-scan-worker' or expected.get('bytes') != result['diskgraph-scan-worker']['bytes'] or expected.get('sha256') != result['diskgraph-scan-worker']['sha256']:
        raise ValueError('worker image does not match package manifest')
    os.fsync(images)
    return result, manifest


def prepare(archive_path, expected_sha256, prefix, service_uid, service_gid, frontend_uids):
    """准备新隔离部署，返回真实对象回执；失败保留未启用的部分树，不重置任何旧槽。"""
    import io
    policy = validate_identities(service_uid, service_gid, frontend_uids)
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise PermissionError('requires the isolated Linux root installer; no desktop fallback')
    if len(expected_sha256) != 64 or any(c not in '0123456789abcdef' for c in expected_sha256):
        raise ValueError('independent lowercase archive SHA256 is required')
    targets = {'aarch64': 'aarch64-unknown-linux-gnu', 'x86_64': 'x86_64-unknown-linux-gnu'}
    target = targets.get(platform.machine())
    if target is None:
        raise ValueError('unsupported native Linux architecture')
    source = os.open(archive_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    prefix_fd = None
    try:
        metadata = os.fstat(source)
        if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= MAX_ARCHIVE_BYTES:
            raise ValueError('source archive must be a bounded ordinary file')
        prefix_fd = create_fresh_prefix(prefix)
        # 先生成 root 独占副本并验证全部原字节；后续解析只读同一原对象，避免源被原地改写。
        with os.fdopen(source, 'rb', closefd=False) as stream:
            copied = write_at(prefix_fd, '.source.tar.gz', stream, MAX_ARCHIVE_BYTES, 0o400)
        if copied['sha256'] != expected_sha256:
            raise ValueError('archive SHA256 mismatch; deployment remains unenabled')
        os.mkdir('images', mode=0o755, dir_fd=prefix_fd)
        images = os.open('images', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=prefix_fd)
        try:
            os.fchmod(images, 0o755)
            archive_fd = os.open('.source.tar.gz', os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=prefix_fd)
            with os.fdopen(archive_fd, 'rb') as snapshot:
                binaries, worker = install_images(snapshot, images, target)
        finally:
            os.close(images)
        os.mkdir('state', mode=0o755, dir_fd=prefix_fd)
        state_fd = os.open('state', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=prefix_fd)
        try:
            os.fchmod(state_fd, 0o755)
            bootstrap = write_at(state_fd, 'bootstrap.slot', io.BytesIO(b'DGSL01C\n'), 8, 0o600)
            service = write_at(state_fd, 'uid_' + str(service_uid) + '.slot', io.BytesIO(b'DGSL01C\n'), 8, 0o600, (service_uid, service_gid))
            state_info = os.fstat(state_fd)
            os.fsync(state_fd)
        finally:
            os.close(state_fd)
        os.mkdir('data', mode=0o700, dir_fd=prefix_fd)
        data_fd = os.open('data', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=prefix_fd)
        try:
            os.fchown(data_fd, service_uid, service_gid)
            os.fchmod(data_fd, 0o700)
            os.fsync(data_fd)
        finally:
            os.close(data_fd)
        receipt = {'schema_version': 1, 'status': 'prepared_not_enabled', 'target': target,
                   'archive_sha256': expected_sha256, 'identity_policy': policy,
                   'role_images': binaries, 'worker_manifest': worker,
                   'state_identity': {'device': state_info.st_dev, 'inode': state_info.st_ino},
                   'bootstrap_slot': bootstrap, 'service_slot': service,
                   'services_started': False, 'broker_authenticated': False,
                   'supervisor_lifecycle_qualified': False}
        write_at(prefix_fd, 'supervisor-deployment.json', io.BytesIO((json.dumps(receipt, indent=2) + '\n').encode()), 16384, 0o644)
        os.unlink('.source.tar.gz', dir_fd=prefix_fd)
        os.fsync(prefix_fd)
        return receipt
    finally:
        os.close(source)
        if prefix_fd is not None:
            os.close(prefix_fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--expected-sha256', required=True)
    parser.add_argument('--prefix', type=Path, required=True)
    parser.add_argument('--service-uid', type=int, required=True)
    parser.add_argument('--service-gid', type=int, required=True)
    parser.add_argument('--frontend-uid', type=int, action='append', required=True)
    args = parser.parse_args()
    receipt = prepare(args.archive, args.expected_sha256, args.prefix, args.service_uid, args.service_gid, args.frontend_uid)
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
