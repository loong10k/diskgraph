use super::git_directory_version::GitDirectoryVersion;
use super::git_metadata_budget::GitMetadataBudget;
use super::probe_budget::ProbeBudget;
use std::ffi::OsString;
use std::fs::File;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

/// 仅在一次目录捕获或复核期间持有完整 no-follow 目录租约。
/// 来源：原生 Rust openat/fdopendir 与 Windows NtCreateFile shareREAD；不是原子快照。
pub(super) struct GitDirectoryLease {
    file: File,
    parents: Vec<File>,
    #[cfg(unix)]
    stream: *mut libc::DIR,
}

impl GitDirectoryLease {
    /// 按原路径逐组件打开并保留全部父句柄。参数：path 为绝对目录，probe 为共享期限/取消。
    /// 返回：短期目录租约；任何父链接、类型、预算及能力错误明确拒绝。
    pub(super) fn open(path: &Path, probe: &mut ProbeBudget) -> Result<Self, String> {
        let (file, parents) = open_directory(path, probe)?;
        Ok(Self {
            file,
            parents,
            #[cfg(unix)]
            stream: std::ptr::null_mut(),
        })
    }

    /// 捕获同一 leaf 句柄的完整目录版本。参数：无。返回：不可变身份版本或查询错误。
    pub(super) fn version(&self) -> Result<GitDirectoryVersion, String> {
        GitDirectoryVersion::capture(&self.file)
    }

    /// 借用租约持有的叶目录句柄，供相对目录的受约束文件操作使用。
    /// 参数：无。返回：借用的目录文件；调用方不能转移句柄或延长租约寿命。
    pub(super) fn leaf_file(&self) -> &File {
        &self.file
    }

    /// 捕获持有祖先的身份记录。参数：probe 为同一次期限/取消。返回：按解析顺序排列的版本，不转移句柄。
    pub(super) fn parent_versions(
        &self,
        probe: &mut ProbeBudget,
    ) -> Result<Vec<GitDirectoryVersion>, String> {
        self.parents
            .iter()
            .map(|file| {
                probe.check().map_err(|error| error.to_string())?;
                GitDirectoryVersion::capture(file)
            })
            .collect()
    }
    /// 由捕获期持有租约枚举名称。参数：path 为精确原路径，budget/probe 为共享额度。返回：有界原生名称或错误。
    #[cfg(unix)]
    pub(super) fn read_names(
        &mut self,
        path: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Vec<OsString>, String> {
        use std::os::fd::{AsRawFd, IntoRawFd};
        let _ = path;
        let mut names = Vec::new();
        use std::os::unix::ffi::OsStringExt;
        let duplicate = self.file.try_clone().map_err(|error| error.to_string())?;
        self.stream = unsafe { libc::fdopendir(duplicate.as_raw_fd()) };
        if self.stream.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // fdopendir 成功后拥有 duplicate fd；Drop 在取消、错误和 unwind 时也关闭它。
        let _ = duplicate.into_raw_fd();
        loop {
            budget.check(probe)?;
            clear_errno();
            let entry = unsafe { libc::readdir(self.stream) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error.to_string());
                }
                break;
            }
            // 内核 dirent 名称以 NUL 终结，下一次 readdir 前复制有界原生名称。
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            budget.charge_entry(probe)?;
            budget.charge_bytes(name.len(), probe)?;
            names.push(OsString::from_vec(name.to_vec()));
        }
        let stream = std::mem::replace(&mut self.stream, std::ptr::null_mut());
        if unsafe { libc::closedir(stream) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(names)
    }

    /// 在 shareREAD 目录及祖先租约存活期间枚举。参数：path 为精确路径，budget/probe 为共享额度。返回：有界名称或错误。
    #[cfg(windows)]
    pub(super) fn read_names(
        &mut self,
        path: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Vec<OsString>, String> {
        use std::os::windows::ffi::OsStrExt;
        let mut names = Vec::new();
        // shareREAD 租约保留所有父目录；前后完整状态复核，不能称为原子枚举。
        for entry in std::fs::read_dir(path)
            .map_err(|error| format!("git metadata directory enumeration: {error}"))?
        {
            budget.check(probe)?;
            let name = entry.map_err(|error| error.to_string())?.file_name();
            budget.charge_entry(probe)?;
            budget.charge_bytes(name.encode_wide().count() * 2, probe)?;
            names.push(name);
        }
        Ok(names)
    }
}

#[cfg(unix)]
impl Drop for GitDirectoryLease {
    fn drop(&mut self) {
        if !self.stream.is_null() {
            unsafe { libc::closedir(self.stream) };
        }
    }
}

#[cfg(unix)]
fn open_directory(path: &Path, probe: &mut ProbeBudget) -> Result<(File, Vec<File>), String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    let raw = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || raw.len() > 32768
        || raw.contains(&0)
        || raw
            .split(|byte| *byte == b'/')
            .any(|part| part == b"." || part == b"..")
    {
        return Err("unsupported git metadata directory path".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(|error| error.to_string())?;
    let mut parents = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        probe.check().map_err(|error| error.to_string())?;
        if parents.len() >= 1024 {
            return Err("git metadata directory depth exceeded".into());
        }
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|error| error.to_string())?;
        let fd = unsafe {
            libc::openat(
                file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let next = unsafe { File::from_raw_fd(fd) };
        parents.push(file);
        file = next;
    }
    probe.check().map_err(|error| error.to_string())?;
    Ok((file, parents))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn clear_errno() {
    unsafe {
        *libc::__errno_location() = 0;
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn clear_errno() {
    unsafe {
        *libc::__error() = 0;
    }
}

#[cfg(windows)]
fn open_directory(path: &Path, probe: &mut ProbeBudget) -> Result<(File, Vec<File>), String> {
    use crate::windows_file_state::WindowsFileState;
    use crate::windows_path_plan::WindowsPathPlan;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, GetDriveTypeW,
    };
    let drive = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Err("unsupported git metadata directory namespace".into()),
        },
        _ => return Err("unsupported git metadata directory path".into()),
    };
    let root = PathBuf::from(format!("{}:\\", drive as char));
    let plan = WindowsPathPlan::new(&root, path).map_err(|error| error.to_string())?;
    let wide: Vec<u16> = plan
        .drive_root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if !matches!(unsafe { GetDriveTypeW(wide.as_ptr()) }, 2 | 3 | 6) {
        return Err("unsupported git metadata directory volume".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(&plan.drive_root)
        .map_err(|error| format!("git metadata drive open: {error}"))?;
    let drive_state = WindowsFileState::capture(&file)
        .map_err(|error| format!("git metadata drive state: {error}"))?;
    drive_state
        .validate(true)
        .map_err(|error| error.to_string())?;
    let mut parents = Vec::new();
    for name in plan.components {
        probe.check().map_err(|error| error.to_string())?;
        let next = open_windows_child(&file, &name)?;
        let state = WindowsFileState::capture(&next)
            .map_err(|error| format!("git metadata child state: {error}"))?;
        state.validate(true).map_err(|error| error.to_string())?;
        if state.volume != drive_state.volume {
            return Err("git metadata directory volume changed".into());
        }
        parents.push(file);
        file = next;
    }
    probe.check().map_err(|error| error.to_string())?;
    Ok((file, parents))
}

#[cfg(windows)]
fn open_windows_child(parent: &File, name: &std::ffi::OsStr) -> Result<File, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_OPEN, FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
        NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let length = u16::try_from(wide.len() * 2).map_err(|error| error.to_string())?;
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
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            // 属性访问不参与共享访问检查；目录读取权限让原 share-read 租约约束删除/改名。
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            FILE_SHARE_READ,
            FILE_OPEN,
            // FILE_DIRECTORY_FILE 只兼容有限的 CreateOptions；与 no-follow/no-recall
            // 同用会得到 STATUS_INVALID_PARAMETER。类型由同一返回句柄校验。
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL,
            std::ptr::null(),
            0,
        )
    };
    if status != 0 || handle.is_null() || handle == INVALID_HANDLE_VALUE {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
        }
        let error =
            std::io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) as i32 });
        return Err(format!("git metadata directory NtCreateFile: {error}"));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}
