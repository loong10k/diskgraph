"""仅验收夹具的 Windows 原父句柄 I/O 候选；未接入正式负载或产品写入口。"""
from contextlib import contextmanager
import ctypes as c
import os
from pathlib import Path
import sys


class UnicodeString(c.Structure):
    """Windows UNICODE_STRING，长度单位为字节，不含终结符。"""
    _fields_ = [('length', c.c_uint16), ('maximum', c.c_uint16), ('buffer', c.c_void_p)]


class ObjectAttributes(c.Structure):
    """Windows OBJECT_ATTRIBUTES；RootDirectory 始终为持有的夹具父句柄。"""
    _fields_ = [('length', c.c_uint32), ('root', c.c_void_p),
                ('name', c.POINTER(UnicodeString)), ('attributes', c.c_uint32),
                ('security', c.c_void_p), ('quality', c.c_void_p)]


class IoStatusValue(c.Union):
    """IO_STATUS_BLOCK 原生联合体，保留指针宽度与 NTSTATUS 宽度。"""
    _fields_ = [('status', c.c_int32), ('pointer', c.c_void_p)]


class IoStatusBlock(c.Structure):
    """同步 NtCreateFile 的完整状态输出，禁止把部分状态当作创建完成。"""
    _fields_ = [('value', IoStatusValue), ('information', c.c_size_t)]


class StandardInformation(c.Structure):
    """FILE_STANDARD_INFO；BOOLEAN 使用一字节，长度与链接数不截断。"""
    _fields_ = [('allocation', c.c_int64), ('length', c.c_int64),
                ('links', c.c_uint32), ('delete_pending', c.c_ubyte), ('directory', c.c_ubyte)]


class AttributeTagInformation(c.Structure):
    """FILE_ATTRIBUTE_TAG_INFO，用于拒绝根或子项的重解析对象。"""
    _fields_ = [('attributes', c.c_uint32), ('tag', c.c_uint32)]


def fixture_name(index):
    """只生成正式夹具已有的固定单组件名称，拒绝额外路径和超量索引。"""
    if isinstance(index, bool) or not isinstance(index, int) or not 0 <= index < 200000:
        raise ValueError('fixture index must be between 0 and 199999')
    return f'file-{index:06}.bin'


class WindowsRelativeFixture:
    """测试根的唯一句柄与显式夹具操作；不用于生产授权或通用删除。"""
    def __init__(self, root):
        if os.name != 'nt':
            raise OSError('native Windows fixture I/O required')
        self.kernel = c.WinDLL('kernel32', use_last_error=True)
        self.nt = c.WinDLL('ntdll', use_last_error=True)
        signatures = {
            'CreateFileW': ([c.c_wchar_p, c.c_uint32, c.c_uint32, c.c_void_p,
                             c.c_uint32, c.c_uint32, c.c_void_p], c.c_void_p),
            'CloseHandle': ([c.c_void_p], c.c_int32),
            'GetFileType': ([c.c_void_p], c.c_uint32),
            'GetFileInformationByHandleEx': ([c.c_void_p, c.c_int32, c.c_void_p, c.c_uint32], c.c_int32),
            'SetFileInformationByHandle': ([c.c_void_p, c.c_int32, c.c_void_p, c.c_uint32], c.c_int32),
            'WriteFile': ([c.c_void_p, c.c_void_p, c.c_uint32, c.POINTER(c.c_uint32), c.c_void_p], c.c_int32),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result
        self.nt.NtCreateFile.argtypes = [c.POINTER(c.c_void_p), c.c_uint32,
            c.POINTER(ObjectAttributes), c.POINTER(IoStatusBlock), c.c_void_p,
            c.c_uint32, c.c_uint32, c.c_uint32, c.c_uint32, c.c_void_p, c.c_uint32]
        self.nt.NtCreateFile.restype = c.c_int32
        self.nt.RtlNtStatusToDosError.argtypes = [c.c_int32]
        self.nt.RtlNtStatusToDosError.restype = c.c_uint32
        self.body = c.create_string_buffer(b'x' * 32, 32)
        # 根由调用方 TemporaryDirectory 新建；这里只打开属性/目录读取，不递归解析子项。
        self.handle = self.kernel.CreateFileW(str(Path(root).absolute()), 0x81, 7, None,
                                               3, 0x02000000 | 0x00200000 | 0x00100000, None)
        if self.handle in (None, c.c_void_p(-1).value):
            self.handle = None
            raise c.WinError(c.get_last_error())
        try:
            standard = self._query(self.handle, 1, StandardInformation)
            tag = self._query(self.handle, 9, AttributeTagInformation)
            if (self.kernel.GetFileType(self.handle) != 1 or not standard.directory
                    or standard.delete_pending or tag.attributes & 0x400):
                raise OSError('unsupported fixture root')
        except BaseException as primary:
            try:
                self.close()
            except OSError as cleanup:
                primary.add_note(f'fixture root close failed: {cleanup}')
            raise

    def _query(self, handle, kind, layout):
        result = layout()
        if not self.kernel.GetFileInformationByHandleEx(handle, kind, c.byref(result), c.sizeof(result)):
            raise c.WinError(c.get_last_error())
        return result

    def _open(self, index, create):
        if self.handle is None:
            raise OSError('fixture root already closed')
        name = fixture_name(index)
        buffer = c.create_unicode_buffer(name)
        length = len(name.encode('utf-16-le'))
        unicode = UnicodeString(length, length, c.cast(buffer, c.c_void_p))
        attributes = ObjectAttributes(c.sizeof(ObjectAttributes), self.handle,
                                      c.pointer(unicode), 0x1000, None, None)
        status_block, handle = IoStatusBlock(), c.c_void_p()
        # FILE_CREATE/FILE_OPEN；同步、非目录、nofollow/no-recall；无覆盖、临时或稀疏属性。
        status = self.nt.NtCreateFile(c.byref(handle), (2 if create else 0x10000) | 0x80 | 0x100000,
            c.byref(attributes), c.byref(status_block), None, 0x80, 7,
            2 if create else 1, 0x20 | 0x40 | 0x00200000 | 0x00400000, None, 0)
        if status != 0 or handle.value in (None, c.c_void_p(-1).value):
            error = (c.WinError(self.nt.RtlNtStatusToDosError(status)) if status else
                     OSError('native fixture open returned no handle'))
            if handle.value not in (None, c.c_void_p(-1).value):
                if not self.kernel.CloseHandle(handle):
                    error.add_note('native fixture failed-open handle close failed')
            raise error
        return handle

    @contextmanager
    def _child(self, index, create):
        handle = self._open(index, create)
        try:
            yield handle
        finally:
            if not self.kernel.CloseHandle(handle):
                error = c.WinError(c.get_last_error())
                primary = sys.exception()
                if primary is None:
                    raise error
                primary.add_note(f'fixture child close failed: {error}')

    def create(self, count):
        """独占创建完整普通文件集合并逐项写满真实 32 字节；错误直接传播。"""
        self._validate_count(count)
        for index in range(count):
            with self._child(index, True) as handle:
                written = c.c_uint32()
                if not self.kernel.WriteFile(handle, self.body, 32, c.byref(written), None):
                    raise c.WinError(c.get_last_error())
                if written.value != 32:
                    raise OSError('incomplete fixture file write')

    def remove(self, count):
        """仅删除已知固定名称的普通单链接对象；不递归、收养目录或跟随链接。"""
        self._validate_count(count)
        for index in range(count):
            with self._child(index, False) as handle:
                standard = self._query(handle, 1, StandardInformation)
                tag = self._query(handle, 9, AttributeTagInformation)
                if (standard.directory or standard.delete_pending or standard.links != 1
                        or tag.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000)):
                    raise OSError('unsupported fixture deletion object')
                delete = c.c_ubyte(1)
                if not self.kernel.SetFileInformationByHandle(handle, 4, c.byref(delete), c.sizeof(delete)):
                    raise c.WinError(c.get_last_error())

    @staticmethod
    def _validate_count(count):
        if isinstance(count, bool) or not isinstance(count, int) or not 1 <= count <= 200000:
            raise ValueError('fixture count must be between 1 and 200000')

    def close(self):
        """关闭同一根句柄；失败保留句柄供调用方处理，不宣称已关闭。"""
        if self.handle is not None:
            if not self.kernel.CloseHandle(self.handle):
                raise c.WinError(c.get_last_error())
            self.handle = None

    def __enter__(self):
        return self

    def __exit__(self, kind, value, traceback):
        try:
            self.close()
        except OSError as error:
            if value is None:
                raise
            value.add_note(f'fixture root close failed: {error}')
        return False
