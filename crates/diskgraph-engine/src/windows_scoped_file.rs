use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;

use diskgraph_core::BusinessError;
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, INVALID_HANDLE_VALUE,
    OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ, GetDriveTypeW, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use crate::EngineError;
use crate::windows_file_state::WindowsFileState;
use crate::windows_path_plan::WindowsPathPlan;

/// 属性阶段的原生目录/文件租约；来源：NtCreateFile RootDirectory 与共享语义。
/// 所有父句柄保留至内容检查结束，先拒绝占位/重解析，再请求数据权限。
pub(crate) struct WindowsScopedFile {
    leaf: File,
    parents: Vec<File>,
    name: std::ffi::OsString,
    pub(crate) state: WindowsFileState,
}

impl WindowsScopedFile {
    /// 按注册根的原始组件获取属性句柄；返回租约，数据读取尚未开始。
    pub(crate) fn open(root: &Path, path: &Path) -> Result<Self, EngineError> {
        let plan = WindowsPathPlan::new(root, path)?;
        let drive: Vec<u16> = plan
            .drive_root
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // GetDriveType 的本地 removable/fixed/ramdisk 为 2/3/6；网络和未知卷拒绝。
        if !matches!(unsafe { GetDriveTypeW(drive.as_ptr()) }, 2 | 3 | 6) {
            return Err(EngineError::Business(BusinessError::Unsupported));
        }
        let drive = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_OPEN_NO_RECALL,
            )
            .open(&plan.drive_root)
            .map_err(native_error)?;
        let drive_state = WindowsFileState::capture(&drive)?;
        drive_state.validate(true)?;
        let mut parents = vec![drive];
        for (index, name) in plan.components.iter().enumerate() {
            let leaf = open_child(parents.last().expect("drive parent exists"), name, false)?;
            let state = WindowsFileState::capture(&leaf)?;
            if state.volume != drive_state.volume {
                return Err(EngineError::Business(BusinessError::Unsupported));
            }
            if index + 1 == plan.components.len() {
                // 占位文件可返回无内容结果，但重解析对象从不交给数据打开步骤。
                state.validate(false)?;
                return Ok(Self {
                    leaf,
                    parents,
                    name: name.clone(),
                    state,
                });
            }
            state.validate(true)?;
            parents.push(leaf);
        }
        Err(EngineError::Business(BusinessError::InvalidArgument))
    }

    /// 从同一持有父目录申请读取数据，并再次核验版本；变化时返回冲突。
    pub(crate) fn open_data(&self) -> Result<File, EngineError> {
        if self.state.placeholder() || WindowsFileState::capture(&self.leaf)? != self.state {
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        let file = open_child(
            self.parents.last().expect("drive parent exists"),
            &self.name,
            true,
        )?;
        if !self.matches(&file) {
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        Ok(file)
    }

    /// 比较数据和属性句柄的完整版本；返回 false 时内容/摘要必须标记不稳定。
    pub(crate) fn matches(&self, file: &File) -> bool {
        WindowsFileState::capture(file).is_ok_and(|state| state == self.state)
            && WindowsFileState::capture(&self.leaf).is_ok_and(|state| state == self.state)
    }

    /// 只从已持有属性句柄取兼容元数据；返回值不经过客户端路径重开。
    pub(crate) fn metadata(&self) -> Result<std::fs::Metadata, EngineError> {
        Ok(self.leaf.metadata()?)
    }
}

fn open_child(parent: &File, name: &OsStr, data: bool) -> Result<File, EngineError> {
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let length = u16::try_from(wide.len() * 2)
        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
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
    let mut status_block = IO_STATUS_BLOCK::default();
    // 安全性：父句柄与 UTF-16 缓冲在调用期间有效；单组件不含路径分隔或 ADS。
    // FILE_OPEN 只打开既有对象；同步模式必须同时请求 SYNCHRONIZE。
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE | if data { FILE_READ_DATA } else { 0 },
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            FILE_SHARE_READ,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT
                | FILE_OPEN_REPARSE_POINT
                | FILE_OPEN_NO_RECALL
                | if data { FILE_NON_DIRECTORY_FILE } else { 0 },
            std::ptr::null(),
            0,
        )
    };
    if status != 0 || handle.is_null() || handle == INVALID_HANDLE_VALUE {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
        }
        return Err(native_error(std::io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status) as i32
        })));
    }
    // 成功句柄立即转给 File 唯一拥有，所有退出路径由 RAII 关闭。
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn native_error(error: std::io::Error) -> EngineError {
    if matches!(error.raw_os_error(), Some(code) if code == ERROR_SHARING_VIOLATION as i32 || code == ERROR_LOCK_VIOLATION as i32)
    {
        return EngineError::Business(BusinessError::Conflict);
    }
    error.into()
}
