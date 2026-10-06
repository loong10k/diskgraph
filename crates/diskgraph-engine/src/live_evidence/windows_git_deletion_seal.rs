use super::git_private_allocation::GitPrivateAllocation;
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ID_INFO, FILE_STANDARD_INFO, FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx,
};

/// 原成功删除标记后的对象资格核验；来源：NTFS原生后置元数据与PF-06，无Java对应。
/// 删除等待不等于完整删除：剩余链接、未知身份或查询失败均保持原恢复责任。
pub(super) struct WindowsGitDeletionSeal;

impl WindowsGitDeletionSeal {
    /// 参数：file为成功标记的原DELETE句柄、expected为原登记身份；返回：后置资格成立或原错误。
    /// 原句柄必须同卷同完整ID、类型一致、处于删除等待且剩余链接为零；不按名称重开。
    pub(super) fn verify(file: &File, expected: &GitPrivateAllocation) -> io::Result<()> {
        let mut id = FILE_ID_INFO::default();
        let mut standard = FILE_STANDARD_INFO::default();
        let id_result = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                std::ptr::addr_of_mut!(id).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        if id_result == 0 {
            return Err(io::Error::last_os_error());
        }
        let standard_result = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                std::ptr::addr_of_mut!(standard).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        };
        if standard_result == 0 {
            return Err(io::Error::last_os_error());
        }
        if !expected.windows_matches_file_identity(id.VolumeSerialNumber, &id.FileId.Identifier)
            || standard.Directory != expected.is_directory()
            || !standard.DeletePending
            || standard.NumberOfLinks != 0
            || standard.AllocationSize < 0
            || standard.EndOfFile < 0
        {
            return Err(io::Error::other(
                "original deletion post-mark identity/type/pending/link seal failed",
            ));
        }
        Ok(())
    }
}
