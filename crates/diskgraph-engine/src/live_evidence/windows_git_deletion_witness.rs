use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use super::windows_git_native_id_protocol::WindowsGitNativeIdProtocol;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_OPEN_BY_FILE_ID, FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT, NtOpenFile,
};
use windows_sys::Win32::Foundation::{
    INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

/// 原完整File ID的最终删除确认；来源：Windows NtOpenFile FILE_OPEN_BY_FILE_ID与PF-06，无Java对等对象。
/// 本对象不删除、不按路径重开、不把访问拒绝或delete-pending当作已回收。
pub(super) struct WindowsGitDeletionWitness;

impl WindowsGitDeletionWitness {
    /// 参数：hint为原卷的可信持有句柄、expected为创建/登记身份、probe为本轮预算。
    /// 返回：明确原ID不存在为true，原对象仍存在为false；等待删除/权限/未知能力均返回原错误。
    /// 单次内核调用无硬时限；调用者在false/Err时必须保留原owner及容量责任。
    pub(super) fn confirm_absent(
        hint: &File,
        expected: &GitPrivateAllocation,
        probe: &mut ProbeBudget,
    ) -> io::Result<bool> {
        probe.check().map_err(io::Error::other)?;
        let volume = GitPrivateAllocation::from_file(hint).map_err(io::Error::other)?;
        if !volume.same_volume(expected) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "deletion witness volume mismatch",
            ));
        }
        if !volume.is_directory() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "deletion witness needs held directory volume hint",
            ));
        }
        // 原hint对象仍保活，先用同一完整ID协议打开正控并核验，未知ID解释不能被当作缺失。
        let protocol = WindowsGitNativeIdProtocol::from_hint(hint);
        probe.check().map_err(io::Error::other)?;
        let protocol = protocol?;
        let control = Self::open_id(hint, &volume, &protocol).map_err(|error| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("full-ID native witness capability unconfirmed: {error}"),
            )
        })?;
        if !volume
            .same_identity(&GitPrivateAllocation::from_file(&control).map_err(io::Error::other)?)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "full-ID native witness control identity mismatch",
            ));
        }
        drop(control);
        probe.check().map_err(io::Error::other)?;
        let file = match Self::open_id(hint, expected, &protocol) {
            Ok(file) => file,
            Err(error) => {
                probe.check().map_err(io::Error::other)?;
                // 参数错误87、权限拒绝5及其他未知错误均不是删除证明。
                return if error.raw_os_error() == Some(2) {
                    Ok(true)
                } else {
                    Err(error)
                };
            }
        };
        let current = GitPrivateAllocation::from_file(&file).map_err(io::Error::other)?;
        if !expected.same_identity(&current) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "deletion witness identity mismatch",
            ));
        }
        probe.check().map_err(io::Error::other)?;
        Ok(false)
    }

    // 参数：原hint和完整ID；返回：只读属性句柄或原NT错误，不映射任何未知错误为absent。
    fn open_id(
        hint: &File,
        expected: &GitPrivateAllocation,
        protocol: &WindowsGitNativeIdProtocol,
    ) -> io::Result<File> {
        let descriptor = expected.windows_file_id_descriptor();
        // 保留完整16字节身份存储，只使用已验证无损的原卷原生操作格式；无错误后回退。
        // descriptor由原固定ExtendedFileIdType方法构造，读取对应union成员；u16存储保证ABI对齐。
        let id = unsafe { descriptor.Anonymous.ExtendedFileId.Identifier };
        let mut binary: [u16; 8] =
            std::array::from_fn(|index| u16::from_ne_bytes([id[index * 2], id[index * 2 + 1]]));
        let length = protocol.byte_length(&id)?;
        let name = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: binary.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: hint.as_raw_handle(),
            ObjectName: &name,
            Attributes: OBJ_DONT_REPARSE,
            ..OBJECT_ATTRIBUTES::default()
        };
        let mut handle = std::ptr::null_mut();
        let mut status_block = IO_STATUS_BLOCK::default();
        let status = unsafe {
            NtOpenFile(
                &mut handle,
                FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                &attributes,
                &mut status_block,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN_BY_FILE_ID
                    | FILE_SYNCHRONOUS_IO_NONALERT
                    | FILE_OPEN_REPARSE_POINT
                    | FILE_OPEN_NO_RECALL,
            )
        };
        // 原返回有效句柄先交RAII，原生错误带句柄也关闭；此操作永不请求DELETE。
        let file = (!handle.is_null() && handle != INVALID_HANDLE_VALUE)
            .then(|| unsafe { File::from_raw_handle(handle) });
        if status != 0 {
            let code = unsafe { RtlNtStatusToDosError(status) as i32 };
            // 原生验收保留转换前状态，区分不同NT错误映射到同一Win32错误；不改变结果语义。
            #[cfg(test)]
            eprintln!(
                "DG_NATIVE_ID_OPEN_STATUS={:#010x}; win32={code}; id_bytes={length}",
                status as u32
            );
            let error = io::Error::from_raw_os_error(code);
            return Err(error);
        }
        file.ok_or_else(|| io::Error::other("deletion witness returned no valid native handle"))
    }
}
