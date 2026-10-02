use super::git_metadata_version::GitMetadataVersion;
use std::fs::File;
use std::path::Path;

/// 私有对象的原生报告分配、类型与完整身份；来源：Unix fstat 或 Windows FileId/StandardInfo。
/// 不以逻辑长度替代分配；文件系统未归属到对象的全局元数据不在此数值内。
pub(super) struct GitPrivateAllocation {
    allocated: u64,
    version: Option<GitMetadataVersion>,
    directory: bool,
    volume: u64,
    #[cfg(unix)]
    id: u64,
    #[cfg(not(unix))]
    id: [u8; 16],
}

impl GitPrivateAllocation {
    /// 不跟随叶链接地捕获文件或目录分配。参数：path 为受控路径。返回：身份分配或原生错误。
    pub(super) fn capture(path: &Path) -> Result<Self, String> {
        let mut options = std::fs::OpenOptions::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
                FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
            };
            options
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(
                    FILE_FLAG_BACKUP_SEMANTICS
                        | FILE_FLAG_OPEN_REPARSE_POINT
                        | FILE_FLAG_OPEN_NO_RECALL,
                );
        }
        let file = options
            .open(path)
            .map_err(|error| format!("private Git allocation open: {error}"))?;
        Self::from_file(&file)
    }

    /// 读取已持有句柄的分配与身份。参数：file 为普通文件/目录句柄。返回：可确认记录；未知能力拒绝。
    pub(super) fn from_file(file: &File) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = file
                .metadata()
                .map_err(|error| format!("private Git allocation stat: {error}"))?;
            if (!metadata.is_file() && !metadata.is_dir())
                || (metadata.is_file() && metadata.nlink() != 1)
            {
                return Err("unsupported private Git allocation object".into());
            }
            let allocated = metadata
                .blocks()
                .checked_mul(512)
                .ok_or("private Git allocation overflow")?;
            Ok(Self {
                allocated,
                version: if metadata.is_file() {
                    Some(GitMetadataVersion::from_metadata(&metadata)?)
                } else {
                    None
                },
                directory: metadata.is_dir(),
                volume: metadata.dev(),
                id: metadata.ino(),
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
                FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
                FILE_ID_INFO, FILE_STANDARD_INFO, FILE_TYPE_DISK, FileBasicInfo, FileIdInfo,
                FileStandardInfo, GetFileInformationByHandleEx, GetFileType,
            };
            let handle = file.as_raw_handle();
            let mut id = FILE_ID_INFO::default();
            let mut standard = FILE_STANDARD_INFO::default();
            let mut basic = FILE_BASIC_INFO::default();
            let valid = unsafe {
                GetFileType(handle) == FILE_TYPE_DISK
                    && GetFileInformationByHandleEx(
                        handle,
                        FileIdInfo,
                        (&mut id as *mut FILE_ID_INFO).cast(),
                        std::mem::size_of::<FILE_ID_INFO>() as u32,
                    ) != 0
                    && GetFileInformationByHandleEx(
                        handle,
                        FileStandardInfo,
                        (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                        std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
                    ) != 0
                    && GetFileInformationByHandleEx(
                        handle,
                        FileBasicInfo,
                        (&mut basic as *mut FILE_BASIC_INFO).cast(),
                        std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                    ) != 0
            };
            if !valid
                || standard.AllocationSize < 0
                || standard.DeletePending
                || (!standard.Directory && standard.NumberOfLinks != 1)
                || basic.FileAttributes
                    & (FILE_ATTRIBUTE_REPARSE_POINT
                        | FILE_ATTRIBUTE_OFFLINE
                        | FILE_ATTRIBUTE_RECALL_ON_OPEN
                        | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
                    != 0
            {
                return Err("unsupported private Git native allocation".into());
            }
            let state = crate::windows_file_state::WindowsFileState::capture(file)
                .map_err(|error| format!("private Git native file version: {error}"))?;
            state
                .validate(standard.Directory)
                .map_err(|error| error.to_string())?;
            if state.placeholder() {
                return Err("unsupported private Git placeholder object".into());
            }
            let version = if standard.Directory {
                None
            } else {
                Some(GitMetadataVersion::from_windows_state(
                    &state,
                    file.metadata()
                        .and_then(|metadata| metadata.modified())
                        .map_err(|error| format!("private Git modified time: {error}"))?,
                ))
            };
            Ok(Self {
                allocated: standard.AllocationSize as u64,
                version,
                directory: standard.Directory,
                volume: id.VolumeSerialNumber,
                id: id.FileId.Identifier,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = file;
            Err("unsupported private Git allocation platform".into())
        }
    }

    /// 取得对象原生报告的分配字节。参数：无。返回：有效零也保留，不代指未知。
    pub(super) fn bytes(&self) -> u64 {
        self.allocated
    }

    /// 确认对象类型。参数：无。返回：目录为 true，普通文件为 false。
    pub(super) fn is_directory(&self) -> bool {
        self.directory
    }

    /// 判断完整身份与类型是否一致。参数：current 为另一持有句柄的记录。返回：同一对象时 true。
    pub(super) fn same_identity(&self, current: &Self) -> bool {
        self.volume == current.volume
            && self.id == current.id
            && self.directory == current.directory
    }

    /// 核对普通文件原生完整版本，不比较会因读取改变的 atime。
    /// 参数：current 为当前句柄记录。返回：身份、长度与高精度修改/变更指纹一致时 true；目录只比较身份。
    pub(super) fn same_version(&self, current: &Self) -> bool {
        self.same_identity(current)
            && match (&self.version, &current.version) {
                (Some(old), Some(now)) => old.same_initial(now),
                (None, None) => self.directory && current.directory,
                _ => false,
            }
    }

    /// 判断对象仍属于原卷。参数：current 为待登记对象。返回：卷身份相同时 true。
    pub(super) fn same_volume(&self, current: &Self) -> bool {
        self.volume == current.volume
    }

    /// 相对于持有父目录独占创建或打开已登记普通文件；不截断现有对象。
    /// 参数：parent/name 为目录句柄和单组件名，create 为独占创建标志。返回：待核对身份的数据句柄。
    pub(super) fn open_write(
        parent: &File,
        name: &std::ffi::OsStr,
        create: bool,
    ) -> Result<File, String> {
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::CString::new(name.as_bytes())
                .map_err(|_| "invalid private Git file name")?;
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDWR
                        | libc::O_NOFOLLOW
                        | libc::O_NONBLOCK
                        | libc::O_CLOEXEC
                        | if create {
                            libc::O_CREAT | libc::O_EXCL
                        } else {
                            0
                        },
                    0o600,
                )
            };
            if fd < 0 {
                return Err(format!(
                    "private Git file open: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(unsafe { File::from_raw_fd(fd) })
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
            if create {
                return open_windows_child(parent, name, true, false, true, FILE_SHARE_READ);
            }
            // 先只申请属性，拒绝占位/离线；保留 no-delete 属性句柄至 data 身份再次确认。
            // 属性 shareWRITE 允许自己的 data-write 重开；data shareREAD 会拒绝正在持有写权限的外部句柄。
            let attributes = open_windows_child(
                parent,
                name,
                false,
                false,
                false,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
            )?;
            let initial = Self::from_file(&attributes)?;
            if initial.is_directory() {
                return Err("private Git write target is a directory".into());
            }
            let file = open_windows_child(parent, name, false, false, true, FILE_SHARE_READ)?;
            if !initial.same_version(&Self::from_file(&file)?)
                || !initial.same_version(&Self::from_file(&attributes)?)
            {
                return Err("private Git file changed before data access".into());
            }
            Ok(file)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (parent, name, create);
            Err("unsupported private Git file platform".into())
        }
    }

    /// 相对于持有父目录打开普通文件进行原生属性核验；不读取内容。
    /// 参数：parent/name 为安全父句柄和单组件名。返回：只读属性/普通文件句柄；链接及特殊类型后续拒绝。
    pub(super) fn open_read(parent: &File, name: &std::ffi::OsStr) -> Result<File, String> {
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::CString::new(name.as_bytes())
                .map_err(|_| "invalid private Git file name")?;
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(format!(
                    "private Git file inspect: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(unsafe { File::from_raw_fd(fd) })
        }
        #[cfg(windows)]
        {
            open_windows_child(
                parent,
                name,
                false,
                false,
                false,
                windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ,
            )
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (parent, name);
            Err("unsupported private Git inspection platform".into())
        }
    }

    /// 关闭写句柄后取得最终属性版本，Windows 时间可能直到 writer 关闭才更新。
    /// 参数：parent/name 为原安全父句柄与组件，file 为已写完句柄。返回：同一身份的只读属性句柄或变化错误。
    pub(super) fn finish_write(
        parent: &File,
        name: &std::ffi::OsStr,
        file: File,
    ) -> Result<File, String> {
        let initial = Self::from_file(&file)?;
        #[cfg(windows)]
        let attributes = {
            use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
            let retained = open_windows_child(
                parent,
                name,
                false,
                false,
                false,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
            )?;
            if !initial.same_identity(&Self::from_file(&retained)?) {
                return Err("private Git file identity changed at write completion".into());
            }
            retained
        };
        drop(file);
        let inspected = Self::open_read(parent, name)?;
        let current = Self::from_file(&inspected)?;
        if !initial.same_identity(&current) {
            return Err("private Git file identity changed at write completion".into());
        }
        #[cfg(unix)]
        if !initial.same_version(&current) {
            return Err("private Git file version changed at write completion".into());
        }
        #[cfg(windows)]
        if !current.same_version(&Self::from_file(&attributes)?) {
            return Err("private Git file version changed at write completion".into());
        }
        Ok(inspected)
    }

    /// 在持有父目录下独占创建目录，权限在创建时生效。
    /// 参数：parent/name 为安全父句柄和单组件名。返回：新目录句柄；既有目标明确冲突。
    pub(super) fn create_directory(parent: &File, name: &std::ffi::OsStr) -> Result<File, String> {
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::CString::new(name.as_bytes())
                .map_err(|_| "invalid private Git directory name")?;
            if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(format!(
                    "private Git directory create: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(format!(
                    "private Git directory open: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(unsafe { File::from_raw_fd(fd) })
        }
        #[cfg(windows)]
        {
            open_windows_child(
                parent,
                name,
                true,
                true,
                false,
                windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ,
            )
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (parent, name);
            Err("unsupported private Git directory platform".into())
        }
    }
}

#[cfg(windows)]
fn open_windows_child(
    parent: &File,
    name: &std::ffi::OsStr,
    create: bool,
    directory: bool,
    write: bool,
    share_mode: u32,
) -> Result<File, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_NO_RECALL,
        FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let mut wide: Vec<u16> = name.encode_wide().take(32768).collect();
    let length =
        u16::try_from(wide.len() * 2).map_err(|_| "unsupported private Git component length")?;
    if wide.is_empty() || wide.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92)) {
        return Err("invalid private Git component".into());
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
    let mut status_block = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES
                | SYNCHRONIZE
                | if write {
                    FILE_READ_DATA | FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES
                } else {
                    0
                },
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            share_mode,
            if create { FILE_CREATE } else { FILE_OPEN },
            FILE_SYNCHRONOUS_IO_NONALERT
                | if directory {
                    // 新目录只能 FILE_CREATE；不会跟随既有目标，DIRFILE 不与 reparse/no-recall 混用。
                    FILE_DIRECTORY_FILE
                } else {
                    FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL | FILE_NON_DIRECTORY_FILE
                },
            std::ptr::null(),
            0,
        )
    };
    if status != 0 || handle.is_null() || handle == INVALID_HANDLE_VALUE {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
        }
        return Err(format!(
            "private Git native create/open: {}",
            std::io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) as i32 })
        ));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}
