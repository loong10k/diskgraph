//! D31 属性专用相对打开；旧内容读取的权限、共享与重解析策略保持独立。
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;

use diskgraph_core::BusinessError;
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_OPEN, FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
    NtCreateFile,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, INVALID_HANDLE_VALUE,
    OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use crate::EngineError;

/// 从已验证的本地 drive 根取得属性句柄，不申请正文或创建权限。
/// 参数：drive_root 为 WindowsPathPlan 产生的 drive 根。
/// 返回：仅属性/同步权限且无 DELETE 共享的根句柄；网络/未知 drive 拒绝。
pub(crate) fn open_drive(drive_root: &Path) -> Result<File, EngineError> {
    let drive: Vec<u16> = drive_root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if !matches!(unsafe { GetDriveTypeW(drive.as_ptr()) }, 2 | 3 | 6) {
        return Err(BusinessError::Unsupported.into());
    }
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(drive_root)
        .map_err(native_error)
}

/// 从保留父句柄打开一个组件的属性；末叶可观察重解析对象本身。
/// 参数：parent 为安全父目录，name 为单个原生名称，deny_reparse 标记父/根策略。
/// 返回：仅属性/同步权限的既有对象句柄；绝不回退完整路径或申请正文权限。
pub(crate) fn open_child(
    parent: &File,
    name: &OsStr,
    deny_reparse: bool,
) -> Result<File, EngineError> {
    let wide: Vec<u16> = name.encode_wide().take(32768).collect();
    open_child_utf16(parent, &wide, deny_reparse)
}

/// 使用预编码的单组件名称执行同一属性打开，不复制或缓存查询结果。
/// 参数：parent为保留父句柄，wide为原生UTF-16名称，deny_reparse为父/根策略。
/// 返回：属性句柄或原错误；名称边界仍逐次验证，输入在同步调用期间只读借用。
pub(crate) fn open_child_utf16(
    parent: &File,
    wide: &[u16],
    deny_reparse: bool,
) -> Result<File, EngineError> {
    if wide.is_empty()
        || wide.len() >= 32768
        || wide == [46]
        || wide == [46, 46]
        || wide.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92))
    {
        return Err(BusinessError::InvalidArgument.into());
    }
    let length = u16::try_from(wide.len() * 2)
        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
    let unicode = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        // Windows输入结构使用可变指针类型，但NtCreateFile的ObjectAttributes只读。
        Buffer: wide.as_ptr().cast_mut(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &unicode,
        Attributes: if deny_reparse { OBJ_DONT_REPARSE } else { 0 },
        ..OBJECT_ATTRIBUTES::default()
    };
    let mut handle = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    // 父句柄、单组件名称及 Unicode 缓冲在同步调用期间有效；FILE_OPEN 不创建。
    // 末叶不设置 DONT_REPARSE，通过 OPEN_REPARSE_POINT 取得链接本身属性。
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
            std::ptr::null(),
            0,
        )
    };
    if status != 0 || handle.is_null() || handle == INVALID_HANDLE_VALUE {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
        }
        // STATUS_REPARSE_POINT_ENCOUNTERED 明确表示父解析被拒，不冒充普通读取失败。
        if status as u32 == 0xc000_050b {
            return Err(BusinessError::Unsupported.into());
        }
        return Err(native_error(std::io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status) as i32
        })));
    }
    // 唯一拥有返回句柄；任何提前退出均由 File 关闭。
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn native_error(error: std::io::Error) -> EngineError {
    if matches!(error.raw_os_error(), Some(code) if code == ERROR_SHARING_VIOLATION as i32 || code == ERROR_LOCK_VIOLATION as i32)
    {
        return BusinessError::Conflict.into();
    }
    error.into()
}
