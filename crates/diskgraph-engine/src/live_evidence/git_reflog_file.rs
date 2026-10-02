//! 原始 Git reflog 的句柄租约；逐组件拒绝链接、非普通文件及不可验证的读取。

use super::probe_budget::ProbeBudget;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// 持有 reflog 文件与初始版本，供两次受预算约束的读取和路径复核。
/// Unix 逐组件 nofollow 后保留文件；Windows 同时保留父目录租约。
/// 来源：原生 Rust diskgraph-engine::live_evidence::git_reflog_file。
pub(super) struct GitReflogFile {
    #[cfg(unix)]
    file: File,
    #[cfg(unix)]
    initial: std::fs::Metadata,
    #[cfg(windows)]
    lease: crate::windows_scoped_file::WindowsScopedFile,
}

impl GitReflogFile {
    /// 安全打开 common 根下的固定 reflog 路径；只有明确 ENOENT 可表示日志不存在。
    /// 参数：path 为 Git common 根追加固定日志后缀的绝对路径，budget 为整次期限和取消。
    /// 返回：普通文件租约、明确不存在或路径/平台/IO 错误。
    pub(super) fn open(path: &Path, budget: &mut ProbeBudget) -> Result<Option<Self>, String> {
        budget.check().map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            let Some(file) = open_unix(path, budget)? else {
                return Ok(None);
            };
            let initial = file
                .metadata()
                .map_err(|error| format!("stash metadata: {error}"))?;
            Ok(Some(Self { file, initial }))
        }
        #[cfg(windows)]
        {
            open_windows(path, budget).map(|lease| lease.map(|lease| Self { lease }))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err("unsupported stash reflog platform".into())
        }
    }

    /// 从租约句柄读取完整原始字节，每块计入整次共享预算并核验身份版本。
    /// 参数：budget 为 Git 子命令与本地文件共用的期限、取消和字节额度。
    /// 返回：原始 reflog 字节；超限或内容/身份变化均返回错误。
    pub(super) fn read_bounded(&mut self, budget: &mut ProbeBudget) -> Result<Vec<u8>, String> {
        #[cfg(unix)]
        {
            use std::io::{Seek, SeekFrom};
            budget.check().map_err(|error| error.to_string())?;
            let before = self
                .file
                .metadata()
                .map_err(|error| format!("stash metadata: {error}"))?;
            if !before.is_file() {
                return Err("stash reflog is not a regular file".into());
            }
            if !same_unix_version(&self.initial, &before) {
                return Err("stash reflog changed before read".into());
            }
            self.file
                .seek(SeekFrom::Start(0))
                .map_err(|error| format!("stash seek: {error}"))?;
            let bytes = read_chunks(&mut self.file, budget)?;
            let after = self
                .file
                .metadata()
                .map_err(|error| format!("stash metadata: {error}"))?;
            if !same_unix_version(&self.initial, &after) || after.len() != bytes.len() as u64 {
                return Err("stash reflog changed during read".into());
            }
            Ok(bytes)
        }
        #[cfg(windows)]
        {
            budget.check().map_err(|error| error.to_string())?;
            let mut data = self
                .lease
                .open_data()
                .map_err(|error| format!("stash data: {error}"))?;
            let bytes = read_chunks(&mut data, budget)?;
            if !self.lease.matches(&data) || self.lease.state.len != bytes.len() as u64 {
                return Err("stash reflog changed during read".into());
            }
            Ok(bytes)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = budget;
            Err("unsupported stash reflog platform".into())
        }
    }

    /// 重新从同一路径获取对象并比较身份版本，识别路径替换和并发改写。
    /// 参数：path 为首次打开的原始绝对路径，budget 为整次期限和取消。
    /// 返回：当前路径仍指向相同版本时 true；其他变化或打开错误明确返回。
    pub(super) fn matches_path(
        &self,
        path: &Path,
        budget: &mut ProbeBudget,
    ) -> Result<bool, String> {
        let Some(current) = Self::open(path, budget)? else {
            return Ok(false);
        };
        #[cfg(unix)]
        {
            let old = self
                .file
                .metadata()
                .map_err(|error| format!("stash metadata: {error}"))?;
            let new = current
                .file
                .metadata()
                .map_err(|error| format!("stash metadata: {error}"))?;
            Ok(same_unix_version(&self.initial, &old) && same_unix_version(&self.initial, &new))
        }
        #[cfg(windows)]
        {
            Ok(self.lease.state == current.lease.state)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = current;
            Err("unsupported stash reflog platform".into())
        }
    }
}

#[cfg(unix)]
fn same_unix_version(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

fn read_chunks(file: &mut File, budget: &mut ProbeBudget) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        budget.check().map_err(|error| error.to_string())?;
        let size = file
            .read(&mut chunk)
            .map_err(|error| format!("stash reflog read: {error}"))?;
        if size == 0 {
            break;
        }
        budget.consume(size).map_err(|error| error.to_string())?;
        bytes.extend_from_slice(&chunk[..size]);
    }
    budget.check().map_err(|error| error.to_string())?;
    Ok(bytes)
}

#[cfg(unix)]
fn open_unix(path: &Path, budget: &mut ProbeBudget) -> Result<Option<File>, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    if !path.is_absolute() {
        return Err("stash path is not absolute".into());
    }
    let mut parent = File::open("/").map_err(|error| format!("stash path root: {error}"))?;
    let mut components = Vec::new();
    for component in path.components() {
        budget.check().map_err(|error| error.to_string())?;
        match component {
            Component::Normal(name) => components.push(name),
            Component::RootDir => {}
            _ => return Err("unsupported stash path component".into()),
        }
    }
    let leaf = components.pop().ok_or("invalid stash reflog path")?;
    for name in components {
        budget.check().map_err(|error| error.to_string())?;
        let name = CString::new(name.as_bytes()).map_err(|_| "invalid stash path component")?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY
                    | libc::O_DIRECTORY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(format!("stash path parent: {error}"));
        }
        // 原生目录句柄保留到下一组件成功打开；每次解析都相对于已确认的父句柄。
        parent = unsafe { File::from_raw_fd(fd) };
    }
    budget.check().map_err(|error| error.to_string())?;
    let leaf = CString::new(leaf.as_bytes()).map_err(|_| "invalid stash path component")?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!("open stash reflog: {error}"));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|error| format!("stash metadata: {error}"))?
        .is_file()
    {
        return Err("stash reflog is not a regular file".into());
    }
    Ok(Some(file))
}

#[cfg(windows)]
fn open_windows(
    path: &Path,
    budget: &mut ProbeBudget,
) -> Result<Option<crate::windows_scoped_file::WindowsScopedFile>, String> {
    use crate::EngineError;
    for _ in path.components() {
        budget.check().map_err(|error| error.to_string())?;
    }
    let root = windows_root(path)?;
    let result = crate::windows_scoped_file::WindowsScopedFile::open(&root, path);
    budget.check().map_err(|error| error.to_string())?;
    match result {
        Ok(lease) => Ok(Some(lease)),
        Err(EngineError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("open stash reflog: {error}")),
    }
}

#[cfg(windows)]
fn windows_root(path: &Path) -> Result<std::path::PathBuf, String> {
    use std::path::Component;

    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Prefix(_)))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err("unsupported stash path root".into());
    }
    // 保留磁盘或 UNC 前缀与根组件；驱动器相对路径不能获得绝对目录权限。
    Ok(path.components().take(2).collect())
}

#[cfg(all(test, windows))]
mod tests {
    use super::windows_root;
    use std::path::Path;

    #[test]
    fn absolute_roots_preserve_native_prefixes() {
        for (path, root) in [
            (r"C:\repo\logs\refs\stash", r"C:\"),
            (r"\\server\share\repo\logs\refs\stash", r"\\server\share\"),
            (r"\\?\C:\repo\logs\refs\stash", r"\\?\C:\"),
            (
                r"\\?\UNC\server\share\repo\logs\refs\stash",
                r"\\?\UNC\server\share\",
            ),
        ] {
            assert_eq!(windows_root(Path::new(path)).unwrap(), Path::new(root));
        }
    }

    #[test]
    fn relative_roots_are_refused() {
        for path in [r"C:repo\logs\refs\stash", r"\repo\logs\refs\stash", "repo"] {
            assert!(windows_root(Path::new(path)).is_err());
        }
    }
}
