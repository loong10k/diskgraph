use super::git_directory_lease::GitDirectoryLease;
use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::PathBuf;
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtOpenFile,
};
use windows_sys::Win32::Foundation::{
    INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFinalPathNameByHandleW, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

/// 原根删除租约的实际父对象核验；来源：PF-06/Windows 原生句柄，无 Java 对等对象。
/// 定位字符串只用于建立无链接组件租约；删除始终作用于原 held root，不作用于路径。
pub(super) struct WindowsGitRootParent;

impl WindowsGitRootParent {
    /// 参数：root 为禁止其他 DELETE 分享的原根租约、expected 为创建身份、probe 为原预算。
    /// 返回：经原根完整 ID 关联核验的实际父句柄；未知命名空间、路径竞态和身份变化均拒绝。
    pub(super) fn bind(
        root: &File,
        expected: &GitPrivateAllocation,
        probe: &mut ProbeBudget,
    ) -> io::Result<File> {
        probe.check().map_err(io::Error::other)?;
        let held = GitPrivateAllocation::from_file(root).map_err(io::Error::other)?;
        if !held.is_directory() || !expected.same_identity(&held) {
            return Err(io::Error::other(
                "original root parent binding identity mismatch",
            ));
        }
        let mut wide = vec![0u16; 32768];
        let length = unsafe {
            GetFinalPathNameByHandleW(
                root.as_raw_handle(),
                wide.as_mut_ptr(),
                wide.len() as u32,
                0,
            )
        } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length >= wide.len() || wide[length] != 0 || wide[..length].contains(&0) {
            return Err(io::Error::other(
                "original root location exceeds native budget",
            ));
        }
        let path = PathBuf::from(OsString::from_wide(&wide[..length]));
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("original root parent unavailable"))?;
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::other("original root component unavailable"))?;
        // 每一父组件由现有 no-reparse/shareREAD 租约固定；根的 DELETE 租约固定成员关联。
        let lease = GitDirectoryLease::open(parent, probe).map_err(io::Error::other)?;
        let parent_identity =
            GitPrivateAllocation::from_file(lease.leaf_file()).map_err(io::Error::other)?;
        if !parent_identity.same_volume(expected) {
            return Err(io::Error::other(
                "original root actual parent volume mismatch",
            ));
        }
        let child = Self::open_component(lease.leaf_file(), name)?;
        let actual = GitPrivateAllocation::from_file(&child).map_err(io::Error::other)?;
        if !expected.same_identity(&actual)
            || !expected
                .same_identity(&GitPrivateAllocation::from_file(root).map_err(io::Error::other)?)
        {
            return Err(io::Error::other(
                "original root actual parent membership changed",
            ));
        }
        probe.check().map_err(io::Error::other)?;
        lease.leaf_file().try_clone()
    }

    // 仅从 held actual parent 打开一个原生名称的属性，不申请 DELETE 或读取正文。
    fn open_component(parent: &File, name: &std::ffi::OsStr) -> io::Result<File> {
        let mut wide: Vec<u16> = name.encode_wide().take(32768).collect();
        if wide.is_empty()
            || wide.len() > 32767
            || wide == [46]
            || wide == [46, 46]
            || wide.iter().any(|u| matches!(*u, 0 | 47 | 58 | 92))
        {
            return Err(io::Error::other("invalid actual parent child component"));
        }
        let length = u16::try_from(wide.len() * 2).map_err(io::Error::other)?;
        let name = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: wide.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle(),
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
                FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
            )
        };
        let file = (!handle.is_null() && handle != INVALID_HANDLE_VALUE)
            .then(|| unsafe { File::from_raw_handle(handle) });
        if status != 0 {
            return Err(io::Error::from_raw_os_error(unsafe {
                RtlNtStatusToDosError(status) as i32
            }));
        }
        file.ok_or_else(|| io::Error::other("actual parent component returned no handle"))
    }
}
