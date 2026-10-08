import ctypes
from ctypes import wintypes
import pathlib
import os


class UnicodeString(ctypes.Structure):
    _fields_ = [('length', wintypes.USHORT), ('maximum', wintypes.USHORT),
                ('buffer', wintypes.LPWSTR)]


class ObjectAttributes(ctypes.Structure):
    _fields_ = [('length', wintypes.ULONG), ('root', wintypes.HANDLE),
                ('name', ctypes.POINTER(UnicodeString)), ('attributes', wintypes.ULONG),
                ('security', ctypes.c_void_p), ('qos', ctypes.c_void_p)]


class IoStatus(ctypes.Structure):
    _fields_ = [('status', ctypes.c_void_p), ('information', ctypes.c_size_t)]


class AttributeTag(ctypes.Structure):
    _fields_ = [('attributes', wintypes.DWORD), ('tag', wintypes.DWORD)]


def create_native(root, files):
    """仅隔离实验：固定目录句柄，独占创建完整32字节文件，不覆盖已有文件。"""
    if os.name != 'nt':
        raise OSError('native Windows experiment only')
    api = ctypes.WinDLL('kernel32', use_last_error=True)
    nt = ctypes.WinDLL('ntdll')
    api.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD,
                                wintypes.HANDLE]
    api.CreateFileW.restype = wintypes.HANDLE
    api.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int,
                                                ctypes.c_void_p, wintypes.DWORD]
    api.GetFileInformationByHandleEx.restype = wintypes.BOOL
    api.WriteFile.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD,
                             ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
    api.WriteFile.restype = wintypes.BOOL
    api.CloseHandle.argtypes = [wintypes.HANDLE]
    api.CloseHandle.restype = wintypes.BOOL
    nt.NtCreateFile.argtypes = [ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD,
                                ctypes.POINTER(ObjectAttributes), ctypes.POINTER(IoStatus),
                                ctypes.c_void_p, wintypes.ULONG, wintypes.ULONG,
                                wintypes.ULONG, wintypes.ULONG, ctypes.c_void_p,
                                wintypes.ULONG]
    nt.NtCreateFile.restype = ctypes.c_long
    nt.RtlNtStatusToDosError.argtypes = [ctypes.c_long]
    nt.RtlNtStatusToDosError.restype = wintypes.ULONG
    # 仅持有自建根；属性/遍历权限，拒绝DELETE共享，不跟随重解析点。
    directory = api.CreateFileW(str(pathlib.Path(root).resolve()), 0x1000A0, 3,
                                None, 3, 0x02200000, None)
    if directory == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        state = AttributeTag()
        if not api.GetFileInformationByHandleEx(directory, 9, ctypes.byref(state),
                                               ctypes.sizeof(state)):
            raise ctypes.WinError(ctypes.get_last_error())
        if not state.attributes & 0x10 or state.attributes & 0x400:
            raise OSError('fixture directory is not a plain native directory')
        payload = ctypes.create_string_buffer(b'x' * 32)
        for index in range(files):
            name = f'file-{index:06}.bin'
            buffer = ctypes.create_unicode_buffer(name)
            text = UnicodeString(len(name) * 2, (len(name) + 1) * 2,
                                 ctypes.cast(buffer, wintypes.LPWSTR))
            attributes = ObjectAttributes(ctypes.sizeof(ObjectAttributes), directory,
                                          ctypes.pointer(text), 0x1040, None, None)
            status = IoStatus()
            handle = wintypes.HANDLE()
            # FILE_CREATE；FILE_NON_DIRECTORY_FILE | SYNCHRONOUS_IO_NONALERT |
            # OPEN_REPARSE_POINT；失败绝不采用已有文件。
            result = nt.NtCreateFile(ctypes.byref(handle), 0x100002,
                                     ctypes.byref(attributes), ctypes.byref(status),
                                     None, 0x80, 3, 2, 0x200060, None, 0)
            if result < 0:
                raise ctypes.WinError(nt.RtlNtStatusToDosError(result))
            try:
                if status.information != 2:
                    raise OSError('native creation did not report FILE_CREATED')
                written = wintypes.DWORD()
                if not api.WriteFile(handle, payload, 32, ctypes.byref(written), None):
                    raise ctypes.WinError(ctypes.get_last_error())
                if written.value != 32:
                    raise OSError('incomplete native fixture write')
            finally:
                if not api.CloseHandle(handle):
                    raise ctypes.WinError(ctypes.get_last_error())
    finally:
        if not api.CloseHandle(directory):
            raise ctypes.WinError(ctypes.get_last_error())
