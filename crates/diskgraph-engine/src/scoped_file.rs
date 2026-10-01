use crate::EngineError;
use diskgraph_core::BusinessError;
#[cfg(unix)]
use std::path::Component;
use std::path::Path;

/// 从注册根目录句柄逐组件打开文件；禁止目录和最终对象中的链接跳转。
#[cfg(unix)]
pub(crate) fn open_scoped(root: &Path, path: &Path) -> Result<std::fs::File, EngineError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    // 允许系统前缀别名（例如 macOS /var）；根以下组件始终保留原始名称。
    let ancestor = path
        .ancestors()
        .find(|ancestor| ancestor.canonicalize().ok().as_deref() == Some(root))
        .ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
    let relative = path
        .strip_prefix(ancestor)
        .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
    let parts = relative.components().collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    }
    let mut directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")?;
    for component in root.components() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => CString::new(name.as_bytes())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?,
            _ => return Err(EngineError::Business(BusinessError::PermissionDenied)),
        };
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        };
        let name = CString::new(name.as_bytes())
            .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
        let is_last = index + 1 == parts.len();
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if is_last {
                libc::O_NONBLOCK
            } else {
                libc::O_DIRECTORY
            };
        // 安全性：CString 有效，目录句柄由本函数持有，新 fd 立即交给 File 唯一拥有。
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let opened = unsafe { std::fs::File::from_raw_fd(fd) };
        if is_last {
            if !opened.metadata()?.is_file() {
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            }
            return Ok(opened);
        }
        directory = opened;
    }
    Err(EngineError::Business(BusinessError::InvalidArgument))
}

/// 无法验证原生目录句柄的平台明确拒绝内容检查。
#[cfg(not(unix))]
pub(crate) fn open_scoped(_root: &Path, _path: &Path) -> Result<std::fs::File, EngineError> {
    Err(EngineError::Business(BusinessError::Unsupported))
}
