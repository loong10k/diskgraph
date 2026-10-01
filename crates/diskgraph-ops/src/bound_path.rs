use crate::OpsError;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::path::{Component, Path};
use std::{ffi::CString, fs::File};

/// 文件名与已固定的父目录句柄，后续打开、发布和清理不再次按完整路径解析。
pub(crate) struct BoundPath {
    parent: File,
    name: CString,
}

impl BoundPath {
    pub(crate) fn open(path: &Path) -> Result<Self, OpsError> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        #[cfg(target_os = "macos")]
        let absolute = if let Ok(rest) = absolute.strip_prefix("/var") {
            Path::new("/private/var").join(rest)
        } else if let Ok(rest) = absolute.strip_prefix("/tmp") {
            Path::new("/private/tmp").join(rest)
        } else {
            absolute
        };
        let name = Self::cstring(
            absolute
                .file_name()
                .ok_or_else(|| OpsError::Stale("path has no name".into()))?,
        )?;
        let mut parent = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        for part in absolute.parent().unwrap_or(Path::new("/")).components() {
            let component = match part {
                Component::RootDir => continue,
                Component::Normal(name) => Self::cstring(name)?,
                _ => return Err(OpsError::Stale("invalid path component".into())),
            };
            parent = Self::open_at(
                &parent,
                &component,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )?;
        }
        Ok(Self { parent, name })
    }

    fn cstring(name: &std::ffi::OsStr) -> Result<CString, OpsError> {
        CString::new(name.as_bytes()).map_err(|_| OpsError::Stale("invalid path name".into()))
    }
    fn open_at(parent: &File, name: &CString, flags: i32) -> Result<File, OpsError> {
        // 安全性：目录和 CString 有效，成功返回的 fd 立即由 File 唯一拥有。
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub(crate) fn read(&self) -> Result<File, OpsError> {
        Self::open_at(
            &self.parent,
            &self.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    }
    pub(crate) fn create(&self) -> Result<File, OpsError> {
        Self::open_at(
            &self.parent,
            &self.name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    }

    /// 在目标父目录句柄中独占创建 staging 目录，返回其中的文件定位。
    pub(crate) fn staging(&self, directory_name: &std::ffi::OsStr) -> Result<Self, OpsError> {
        let directory_name = Self::cstring(directory_name)?;
        if unsafe { libc::mkdirat(self.parent.as_raw_fd(), directory_name.as_ptr(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let parent = Self::open_at(
            &self.parent,
            &directory_name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
        Ok(Self {
            parent,
            name: self.name.clone(),
        })
    }

    /// 使用两个固定的父目录句柄原子发布，既有目标永不覆盖。
    pub(crate) fn rename_to(&self, target: &Self) -> Result<(), OpsError> {
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                target.parent.as_raw_fd(),
                target.name.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                target.parent.as_raw_fd(),
                target.name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EEXIST) {
            return Err(OpsError::TargetExists);
        }
        Err(error.into())
    }

    /// 将源身份与原子操作绑定；发布后核对对象，竞态时禁止覆盖地回滚。
    pub(crate) fn rename_verified_to(
        &self,
        target: &Self,
        expected: &std::fs::Metadata,
    ) -> Result<(), OpsError> {
        use std::os::unix::fs::MetadataExt;
        let same = |current: &std::fs::Metadata, check_ctime: bool| {
            current.dev() == expected.dev()
                && current.ino() == expected.ino()
                && current.len() == expected.len()
                && current.mtime() == expected.mtime()
                && current.mtime_nsec() == expected.mtime_nsec()
                && (!check_ctime
                    || (current.ctime() == expected.ctime()
                        && current.ctime_nsec() == expected.ctime_nsec()))
        };
        let pinned = self.read()?;
        if !same(&pinned.metadata()?, true) {
            return Err(OpsError::Stale(
                "source changed before atomic publication".into(),
            ));
        }
        self.rename_to(target)?;
        let published = target
            .read()
            .and_then(|file| file.metadata().map_err(OpsError::from));
        if published
            .as_ref()
            .is_ok_and(|metadata| same(metadata, false))
        {
            return Ok(());
        }
        if target.rename_to(self).is_ok() {
            return Err(OpsError::Stale(
                "source changed during publication; rolled back without overwrite".into(),
            ));
        }
        Err(OpsError::ParkNeedsAttention(
            "source raced during publication and no-replace rollback conflicted".into(),
        ))
    }

    /// 先转入独占目录并核对身份，再按固定父句柄删除源，拒绝目录递归删除。
    pub(crate) fn remove_verified(&self, expected: &std::fs::Metadata) -> Result<(), OpsError> {
        if !expected.is_file() {
            return Err(OpsError::Stale(
                "unsupported: verified removal of a non-file".into(),
            ));
        }
        let name = format!(".dg-source-removal-{}", uuid::Uuid::new_v4().simple());
        let held = self.staging(std::ffi::OsStr::new(&name))?;
        if let Err(error) = self.rename_verified_to(&held, expected) {
            self.remove_directory(std::ffi::OsStr::new(&name));
            return Err(error);
        }
        if unsafe { libc::unlinkat(held.parent.as_raw_fd(), held.name.as_ptr(), 0) } != 0 {
            if held.rename_to(self).is_ok() {
                self.remove_directory(std::ffi::OsStr::new(&name));
                return Err(std::io::Error::last_os_error().into());
            }
            return Err(OpsError::ParkNeedsAttention(format!(
                "source retained in {name}; removal and no-replace restoration conflicted"
            )));
        }
        self.remove_directory(std::ffi::OsStr::new(&name));
        Ok(())
    }

    pub(crate) fn discard(&self) {
        unsafe {
            libc::unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0);
        }
    }
    pub(crate) fn remove_directory(&self, directory_name: &std::ffi::OsStr) {
        if let Ok(name) = Self::cstring(directory_name) {
            unsafe {
                libc::unlinkat(self.parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR);
            }
        }
    }
}
