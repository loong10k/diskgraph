//! Windows 源目录相对句柄访问；来源：NtCreateFile / FileFullDirectoryInfo。
use super::git_directory_version::GitDirectoryVersion;
use super::git_metadata_budget::GitMetadataBudget;
#[cfg(test)]
use super::git_source_windows_diagnostic::GitSourceWindowsDiagnostic;
use super::probe_budget::ProbeBudget;
use crate::windows_file_state::WindowsFileState;
use crate::windows_path_plan::WindowsPathPlan;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

/// 参数：path 为已注册本地 drive 根路径，probe 为原期限；返回：逐组件 no-reparse/no-recall 枚举能力及完整路径身份链。
pub(super) fn root(
    path: &Path,
    probe: &mut ProbeBudget,
) -> Result<(File, Vec<GitDirectoryVersion>), String> {
    let plan = WindowsPathPlan::for_root(path).map_err(|e| e.to_string())?;
    let wide: Vec<_> = plan
        .drive_root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if !matches!(
        unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) },
        2 | 3 | 6
    ) {
        return Err("unsupported scoped Git source volume".into());
    }
    let open_drive = |access| {
        std::fs::OpenOptions::new()
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_OPEN_NO_RECALL,
            )
            .open(&plan.drive_root)
            .map_err(|e| e.to_string())
    };
    let attributes = open_drive(FILE_READ_ATTRIBUTES)?;
    let initial = state(&attributes, true)?;
    let mut file = open_drive(FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY)?;
    if state(&file, true)? != initial {
        return Err("scoped Git drive changed".into());
    }
    let mut route = vec![GitDirectoryVersion::capture(&file)?];
    for name in plan.components {
        probe.check().map_err(|e| e.to_string())?;
        file = child(&file, &name, true)?;
        route.push(GitDirectoryVersion::capture(&file)?);
    }
    Ok((file, route))
}

fn state(file: &File, directory: bool) -> Result<WindowsFileState, String> {
    let state = WindowsFileState::capture(file).map_err(|e| e.to_string())?;
    state.validate(directory).map_err(|e| e.to_string())?;
    if state.placeholder() {
        return Err("unsupported scoped Git placeholder source".into());
    }
    Ok(state)
}

/// 参数：parent/name 为锚定父句柄及单名称，directory 指定类型；返回：属性验证后才申请枚举或正文的句柄。
pub(super) fn child(parent: &File, name: &OsStr, directory: bool) -> Result<File, String> {
    let attributes = open_child(parent, name, FILE_READ_ATTRIBUTES)?;
    let initial = state(&attributes, directory)?;
    let file = open_child(
        parent,
        name,
        FILE_READ_ATTRIBUTES
            | if directory {
                FILE_LIST_DIRECTORY
            } else {
                FILE_READ_DATA
            },
    )?;
    // 保留原 || 的短路顺序：首个状态不同便拒绝，绝不为诊断追加第二次属性查询。
    let reopened = state(&file, directory)?;
    if reopened != initial {
        #[cfg(test)]
        GitSourceWindowsDiagnostic::record(&initial, &reopened, directory, "reopened");
        return Err("scoped Git source changed before data access".into());
    }
    let rechecked = state(&attributes, directory)?;
    if rechecked != initial {
        #[cfg(test)]
        GitSourceWindowsDiagnostic::record(&initial, &rechecked, directory, "attributes_recheck");
        return Err("scoped Git source changed before data access".into());
    }
    Ok(file)
}

/// 参数：parent/name 为锚定父句柄及名称；返回：仅查询属性后的普通目录分类，不获取正文/枚举权限。
pub(super) fn is_directory(parent: &File, name: &OsStr) -> Result<bool, String> {
    let attributes = open_child(parent, name, FILE_READ_ATTRIBUTES)?;
    let observed = WindowsFileState::capture(&attributes).map_err(|e| e.to_string())?;
    state(&attributes, observed.directory)?;
    Ok(observed.directory)
}

fn open_child(parent: &File, name: &OsStr, access: u32) -> Result<File, String> {
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_OPEN, FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
        NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let mut wide: Vec<u16> = name.encode_wide().take(32768).collect();
    let length =
        u16::try_from(wide.len() * 2).map_err(|_| "unsupported scoped Git component length")?;
    if wide.is_empty()
        || wide == [46]
        || wide == [46, 46]
        || wide.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
    {
        return Err("unsupported scoped Git source component".into());
    }
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
        ..OBJECT_ATTRIBUTES::default()
    };
    let mut handle = std::ptr::null_mut();
    let mut io = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut io,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
            std::ptr::null(),
            0,
        )
    };
    if status != 0 || handle.is_null() || handle == INVALID_HANDLE_VALUE {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(handle);
            }
        }
        return Err(format!(
            "scoped Git relative open: {}",
            std::io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) as i32 })
        ));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// 参数：file 为含 FILE_LIST_DIRECTORY 的已验证句柄，budget/probe 为共享预算；返回：同句柄枚举名单，无路径回退。
pub(super) fn names(
    file: &File,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<Vec<OsString>, String> {
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, GetLastError};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FULL_DIR_INFO, FileFullDirectoryInfo, FileFullDirectoryRestartInfo,
        GetFileInformationByHandleEx,
    };
    let mut buffer = [0u64; 2048];
    let mut names = Vec::new();
    let mut restart = true;
    loop {
        budget.check(probe)?;
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                if restart {
                    FileFullDirectoryRestartInfo
                } else {
                    FileFullDirectoryInfo
                },
                buffer.as_mut_ptr().cast(),
                std::mem::size_of_val(&buffer) as u32,
            )
        };
        restart = false;
        if ok == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_FILES {
                break;
            }
            return Err(format!(
                "scoped Git handle enumeration: {}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), std::mem::size_of_val(&buffer))
        };
        let mut offset = 0usize;
        loop {
            let prefix = std::mem::offset_of!(FILE_FULL_DIR_INFO, FileName);
            if offset
                .checked_add(std::mem::size_of::<FILE_FULL_DIR_INFO>())
                .is_none_or(|end| end > bytes.len())
            {
                return Err("invalid native Git directory record".into());
            }
            let row = unsafe { &*bytes.as_ptr().add(offset).cast::<FILE_FULL_DIR_INFO>() };
            let len = row.FileNameLength as usize;
            let next = row.NextEntryOffset as usize;
            if !len.is_multiple_of(2)
                || offset + prefix + len > bytes.len()
                || (next != 0 && (next < prefix + len || !next.is_multiple_of(8)))
            {
                return Err("invalid native Git directory name".into());
            }
            let units = unsafe {
                std::slice::from_raw_parts(
                    bytes.as_ptr().add(offset + prefix).cast::<u16>(),
                    len / 2,
                )
            };
            if units != [46] && units != [46, 46] {
                budget.charge_entry(probe)?;
                budget.charge_bytes(len, probe)?;
                names.push(OsString::from_wide(units));
            }
            if next == 0 {
                break;
            }
            offset = offset
                .checked_add(next)
                .ok_or("native Git directory offset overflow")?;
        }
    }
    names.sort();
    Ok(names)
}
