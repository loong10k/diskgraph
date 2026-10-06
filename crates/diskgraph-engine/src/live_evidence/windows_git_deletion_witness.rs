use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OpenFileById,
};

/// 原完整File ID的最终删除确认；来源：Windows OpenFileById与PF-06，无Java对等对象。
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
        let descriptor = expected.windows_file_id_descriptor();
        // 按ID仅作存在观察；DELETE权限永不请求，名称替换不会成为新的删除目标。
        let handle = unsafe {
            OpenFileById(
                hint.as_raw_handle(),
                &descriptor,
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_OPEN_NO_RECALL,
            )
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            let error = io::Error::last_os_error();
            probe.check().map_err(io::Error::other)?;
            // ERROR_ACCESS_DENIED(5)包含delete-pending，任何其他未知错误都不能释放责任。
            return if error.raw_os_error() == Some(2) {
                Ok(true)
            } else {
                Err(error)
            };
        }
        let file = unsafe { File::from_raw_handle(handle) };
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
}
