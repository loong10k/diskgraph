"""root 记录与普通 runner 工作目录隔离；读取原句柄并有界读取非链接材料。"""
import json
import os
import stat
import sys


class NativeNamespaceArtifacts:
    """来源：原 namespace supervisor 的日志/回执边界；不改变进程所有权。"""

    @staticmethod
    def close_preserving(fd, primary):
        """关闭次错不得覆盖此前原异常；无原异常时传播实际关闭失败。"""
        try:
            os.close(fd)
        except BaseException as error:
            if primary is None:
                raise
            primary.add_note(f"artifact close secondary: {type(error).__name__} errno={getattr(error, 'errno', None)}")

    @staticmethod
    def initialize(supervisor, parent_fd=None, name=None):
        """持原 root 记录 dirfd；runner 即使替换祖先路径也不能重定向日志/回执。"""
        owned_parent = parent_fd is None
        if owned_parent:
            supervisor.output.parent.mkdir(parents=True, exist_ok=True)
            parent_fd = NativeNamespaceArtifacts.open_directory(supervisor.output.parent)
            name = supervisor.output.name
        try:
            os.mkdir(name, 0o755, dir_fd=parent_fd)
            fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                         dir_fd=parent_fd)
            supervisor.artifact_dir_fd = fd
            metadata = os.fstat(fd)
            if metadata.st_uid != os.geteuid() or metadata.st_mode & 0o022:
                raise PermissionError('record directory must belong exclusively to the outer owner')
            if owned_parent:
                os.mkdir('runner', 0o700, dir_fd=fd)
                os.chown('runner', supervisor.args.runner_uid, supervisor.args.runner_gid,
                         dir_fd=fd, follow_symlinks=False)
        finally:
            if owned_parent:
                NativeNamespaceArtifacts.close_preserving(parent_fd, sys.exception())

    @staticmethod
    def open_directory(path):
        """逐组件持真实目录句柄，任何祖先链接立即拒绝。"""
        directory = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        try:
            for component in path.absolute().parts[1:]:
                child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                dir_fd=directory)
                os.close(directory)
                directory = child
            return os.dup(directory)
        finally:
            NativeNamespaceArtifacts.close_preserving(directory, sys.exception())

    @staticmethod
    def open_log(supervisor, channel):
        """仅固定 stdout/stderr，独占创建原目录日志 FD，不再次解析输出路径。"""
        if channel not in ('stdout', 'stderr'):
            raise ValueError('unknown fixed capture channel')
        fd = os.open('child.' + channel, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                     0o644, dir_fd=supervisor.artifact_dir_fd)
        return os.fdopen(fd, 'w+b')

    @staticmethod
    def close(supervisor):
        """回执收尾之后释放独占记录句柄，不影响原 init/pidfd 所有权。"""
        fd = getattr(supervisor, 'artifact_dir_fd', None)
        if fd is not None:
            os.close(fd)
            supervisor.artifact_dir_fd = None

    @staticmethod
    def capture(stream, size):
        """参数：原打开日志句柄与真实计数；返回：有界内容，绝不重新按路径打开。"""
        if stream.closed:
            raise ValueError('original capture descriptor already closed')
        stream.flush()
        stream.seek(0)
        raw = stream.read((64 << 20) + 1)
        if len(raw) != size or len(raw) > (64 << 20):
            raise ValueError('original capture byte accounting mismatch')
        return raw

    @staticmethod
    def open_read(path):
        """逐组件用真实目录 FD 禁止链接，父目录替换不得引导 root 跟随链接。"""
        parts = path.absolute().parts
        directory = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        try:
            for component in parts[1:-1]:
                child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                dir_fd=directory)
                os.close(directory)
                directory = child
            return os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                           dir_fd=directory)
        finally:
            NativeNamespaceArtifacts.close_preserving(directory, sys.exception())

    @staticmethod
    def bounded_read(path, limit):
        """参数：runner 材料路径和上限；返回：原实际常规 FD 内容，链接/FIFO 立即拒绝。"""
        fd = NativeNamespaceArtifacts.open_read(path)
        try:
            metadata = os.fstat(fd)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > limit:
                raise ValueError('runner receipt is not a bounded actual regular descriptor')
            blocks, total = [], 0
            while block := os.read(fd, min(64 << 10, limit + 1 - total)):
                total += len(block)
                if total > limit:
                    raise ValueError('runner receipt grew beyond its admitted bound')
                blocks.append(block)
            return b''.join(blocks)
        finally:
            NativeNamespaceArtifacts.close_preserving(fd, sys.exception())

    @staticmethod
    def save(supervisor):
        """参数：原 root owner；返回：独占临时文件原子替换回执，不跟随既有目标链接。"""
        path = supervisor.output / 'namespace-receipt.pending'
        held = getattr(supervisor, 'artifact_dir_fd', None)
        directory = os.dup(held) if held is not None else NativeNamespaceArtifacts.open_directory(supervisor.output)
        metadata = os.fstat(directory)
        if metadata.st_uid != os.geteuid() or metadata.st_mode & 0o022:
            os.close(directory)
            raise PermissionError('record directory must belong exclusively to the outer owner')
        try:
            fd = os.open(path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                         0o644, dir_fd=directory)
        except BaseException as error:
            NativeNamespaceArtifacts.close_preserving(directory, error)
            raise
        try:
            stream = None
            original = None
            try:
                stream = os.fdopen(fd, 'w')
                stream.write(json.dumps(supervisor.receipt, indent=2) + '\n')
                stream.flush()
            except BaseException as error:
                original = error
                raise
            finally:
                if stream is None:
                    NativeNamespaceArtifacts.close_preserving(fd, original)
                else:
                    try:
                        stream.close()
                    except BaseException as error:
                        if original is None:
                            raise
                        original.add_note(f"receipt stream close secondary: {type(error).__name__} errno={getattr(error, 'errno', None)}")
            os.replace(path.name, 'namespace-receipt.json', src_dir_fd=directory, dst_dir_fd=directory)
        finally:
            primary = sys.exception()
            try:
                os.unlink(path.name, dir_fd=directory)
            except FileNotFoundError:
                pass
            except BaseException as error:
                if primary is None:
                    raise
                primary.add_note(f"artifact unlink secondary: {type(error).__name__} errno={getattr(error, 'errno', None)}")
            finally:
                NativeNamespaceArtifacts.close_preserving(directory, primary or sys.exception())
