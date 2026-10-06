//! 原子父句柄相对目录创建及原对象重开；尚未接入产品目录清理。
use super::git_directory_security::GitDirectorySecurity;
use super::git_private_allocation::GitPrivateAllocation;
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile, NtOpenFile,
};
use windows_sys::Win32::Foundation::{
    INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

/// 从原子创建起持有的唯一根对象；来源：Windows NtCreateFile/NtOpenFile 与 PF-06。
/// identity=None 是创建后验证失败的 prepared 状态，保留句柄但不具备删除资格。
/// 空名称相对原 anchor 重开，前后核验完整 File ID；没有按路径回退。
pub(super) struct WindowsGitPrivateRoot {
    file: File,
    parent_identity: GitPrivateAllocation,
    creation_confirmed: bool,
    identity: Option<GitPrivateAllocation>,
}
impl WindowsGitPrivateRoot {
    /// 参数：可信 held parent、单组件名、原受保护 DACL、catch 外唯一外槽。
    /// 返回：原子创建并核验成功，或原错误；取得的句柄在失败时仍留原槽。
    /// 外槽非空时拒绝；本方法不删除目录，也不宣称 OS 单调用有硬期限。
    pub(super) fn create_into(
        parent: &File,
        name: &OsStr,
        security: &GitDirectorySecurity,
        owner: &mut Option<Self>,
    ) -> io::Result<()> {
        if owner.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "root owner slot already occupied",
            ));
        }
        let parent_identity = allocation(parent)?;
        if !parent_identity.is_directory() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "root parent is not a directory",
            ));
        }
        // UNICODE_STRING 长度以字节计；先限制迭代，拒绝截断、特殊组件及路径语法。
        let mut wide: Vec<u16> = name.encode_wide().take(32768).collect();
        if wide.is_empty()
            || wide.len() > 32767
            || wide == [46]
            || wide == [46, 46]
            || wide.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid private root component",
            ));
        }
        let length = u16::try_from(wide.len() * 2).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "private root component too long",
            )
        })?;
        let unicode = UNICODE_STRING {
            Length: length,
            MaximumLength: length,
            Buffer: wide.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle(),
            ObjectName: &unicode,
            Attributes: OBJ_DONT_REPARSE,
            SecurityDescriptor: security.descriptor().cast(),
            ..OBJECT_ATTRIBUTES::default()
        };
        let mut handle = std::ptr::null_mut();
        let mut status_block = IO_STATUS_BLOCK::default();
        // windows_sys 的 ABI 提供原生布局和对齐。同步标志+SYNCHRONIZE 不提交局部异步IO；
        // Unicode/DACL/parent 在调用返回前持续存活，FILE_CREATE 永不收养已存在对象。
        let status = unsafe {
            NtCreateFile(
                &mut handle,
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | SYNCHRONIZE,
                &attributes,
                &mut status_block,
                std::ptr::null(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_CREATE,
                FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
                std::ptr::null(),
                0,
            )
        };
        // 在状态投影及任何身份查询前接管返回的有效句柄；失败带句柄也保留prepared owner。
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            *owner = Some(Self {
                file: unsafe { File::from_raw_handle(handle) },
                parent_identity,
                creation_confirmed: status == 0,
                identity: None,
            });
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(unsafe {
                RtlNtStatusToDosError(status) as i32
            }));
        }
        let root = owner.as_mut().ok_or_else(|| {
            io::Error::other("NtCreateFile succeeded without a valid root handle")
        })?;
        root.confirm_created()
    }

    /// 参数：无；返回：通过原 anchor 确认创建对象的目录类型、原父卷及已知身份。
    /// NT 创建成功但查询暂时失败时可再次调用；不按路径重开、不丢 prepared owner。
    /// 原 NT 调用失败即使带回句柄也永不自动提升为已创建对象。
    pub(super) fn confirm_created(&mut self) -> io::Result<()> {
        if !self.creation_confirmed {
            return Err(io::Error::other("native root creation was not confirmed"));
        }
        let identity = allocation(&self.file)?;
        if !identity.is_directory() || !self.parent_identity.same_volume(&identity) {
            return Err(io::Error::other(
                "created private root type or volume mismatch",
            ));
        }
        if self
            .identity
            .as_ref()
            .is_some_and(|expected| !expected.same_identity(&identity))
        {
            return Err(io::Error::other("held private root identity changed"));
        }
        // 仅在全部原对象核验成功后建立或刷新身份，不覆盖失败前已知记录。
        self.identity = Some(identity);
        Ok(())
    }

    /// 参数：无；返回：原 anchor 的受寿命约束借用，仅供持有对象核验/相对操作。
    /// 不提供路径重开、所有权移出或 Clone；prepared 状态仍须由调用者保留。
    pub(super) fn as_file(&self) -> &File {
        &self.file
    }

    /// 参数：无；返回：经原 FileID/卷核验、具有 DELETE 权限的同对象句柄。
    /// 原 anchor 始终保留；失败不按路径重试，不执行删除，不修改 cleaned 状态。
    pub(super) fn reopen_for_delete(&self) -> io::Result<File> {
        let expected = self
            .identity
            .as_ref()
            .ok_or_else(|| io::Error::other("private root identity is unconfirmed"))?;
        let held = allocation(&self.file)?;
        if !held.is_directory() || !expected.same_identity(&held) {
            return Err(io::Error::other("held private root identity changed"));
        }
        // 空名称相对原句柄重开同一对象，不使用按ID打开的删除语义，
        // 也不解析原路径、当前名称或任何可被替换的父目录入口。
        let empty_name = UNICODE_STRING::default();
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: self.file.as_raw_handle(),
            ObjectName: &empty_name,
            Attributes: OBJ_DONT_REPARSE,
            SecurityDescriptor: std::ptr::null_mut(),
            SecurityQualityOfService: std::ptr::null_mut(),
        };
        let mut handle = std::ptr::null_mut();
        let mut status_block = IO_STATUS_BLOCK::default();
        let status = unsafe {
            NtOpenFile(
                &mut handle,
                DELETE | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | SYNCHRONIZE,
                &attributes,
                &mut status_block,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                // DIRECTORY_FILE 不兼容 no-follow/no-recall 选项组合；
                // 保留防护，由原对象和新句柄的身份及目录类型检查约束重开。
                FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
            )
        };
        // 在任何状态投影前接管有效返回句柄；失败返回句柄也由 File 自动关闭。
        let file = (!handle.is_null() && handle != INVALID_HANDLE_VALUE)
            .then(|| unsafe { File::from_raw_handle(handle) });
        if status != 0 {
            return Err(io::Error::from_raw_os_error(unsafe {
                RtlNtStatusToDosError(status) as i32
            }));
        }
        let file = file.ok_or_else(|| io::Error::other("NtOpenFile returned no valid handle"))?;
        let reopened = allocation(&file)?;
        let held_after = allocation(&self.file)?;
        if !reopened.is_directory()
            || !held_after.is_directory()
            || !expected.same_identity(&reopened)
            || !expected.same_identity(&held_after)
        {
            return Err(io::Error::other("reopened private root identity mismatch"));
        }
        Ok(file)
    }
}

fn allocation(file: &File) -> io::Result<GitPrivateAllocation> {
    GitPrivateAllocation::from_file(file).map_err(io::Error::other)
}
