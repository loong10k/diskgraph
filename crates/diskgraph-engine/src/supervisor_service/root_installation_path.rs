use std::ffi::CString;
use std::fs::{File, Metadata};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::time::Instant;

/// 参数：path为固定root安装路径，deadline为原期限；返回：普通原File和原目录链或拒绝。
/// 不跟随任何链接，不创建或修复部署；执行位和摘要由镜像层继续验证。
pub(super) fn open(path: &Path, deadline: Instant) -> io::Result<(File, Vec<File>)> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(denied("root fixed installation required"));
    }
    open_raw(path, deadline, false)
}

/// 参数：path为固定root保护目录，deadline为原期限；返回：原目录及原祖先链或拒绝。
/// 允许普通前端只读认证socket祖先；不创建目录，也不授予root或文件写权限。
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) fn open_directory(path: &Path, deadline: Instant) -> io::Result<(File, Vec<File>)> {
    open_raw(path, deadline, true)
}

/// 参数：path为公开root安装文件、deadline为原期限；返回：原普通文件与不可写目录链。
/// 不要求调用者为root；文件仍须root所有、单链接、不允许组或其他用户写入。
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub(super) fn open_public(path: &Path, deadline: Instant) -> io::Result<(File, Vec<File>)> {
    open_raw(path, deadline, false)
}

fn open_raw(path: &Path, deadline: Instant, leaf_directory: bool) -> io::Result<(File, Vec<File>)> {
    check(deadline)?;
    let text = path
        .to_str()
        .ok_or_else(|| denied("noncanonical installation path"))?;
    if !path.is_absolute() || path.as_os_str().len() > 1024 {
        return Err(denied("root fixed installation required"));
    }
    if text.contains('\0')
        || text[1..]
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(denied("noncanonical installation path"));
    }
    let raw = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut directories = vec![unsafe { File::from_raw_fd(raw) }];
    verify_directory(&directories[0], deadline)?;
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        if component == Component::RootDir {
            continue;
        }
        let Component::Normal(name) = component else {
            return Err(denied("noncanonical installation path"));
        };
        let name =
            CString::new(name.as_bytes()).map_err(|_| denied("invalid installation path"))?;
        check(deadline)?;
        let directory = components.peek().is_some() || leaf_directory;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        let raw = unsafe {
            libc::openat(
                directories.last().expect("original root").as_raw_fd(),
                name.as_ptr(),
                flags,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_fd(raw) };
        if directory {
            verify_directory(&file, deadline)?;
            if components.peek().is_none() {
                return Ok((file, directories));
            }
            directories.push(file);
        } else {
            verify_file(&file, deadline)?;
            return Ok((file, directories));
        }
    }
    Err(denied("installation file missing"))
}

fn verify_file(file: &File, deadline: Instant) -> io::Result<()> {
    check(deadline)?;
    let metadata = file.metadata()?;
    check(deadline)?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
    {
        return Err(denied("untrusted installation file"));
    }
    Ok(())
}
fn verify_directory(file: &File, deadline: Instant) -> io::Result<()> {
    check(deadline)?;
    let metadata = file.metadata()?;
    check(deadline)?;
    if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(denied("untrusted installation ancestor"));
    }
    Ok(())
}
/// 参数：metadata为原句柄元数据；返回：含高精度修改信息的原身份快照。
pub(super) fn identity(metadata: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64, u32, u32, u64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
        metadata.uid(),
        metadata.mode(),
        metadata.nlink(),
    )
}
/// 参数：deadline为原期限；返回：仍有效或TimedOut。
pub(super) fn check(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "installation admission deadline",
        ));
    }
    Ok(())
}

/// 参数：message为固定拒绝说明；返回：PermissionDenied错误。
pub(super) fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
