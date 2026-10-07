//! 当前本地用户的固定持久恢复域；不使用请求、数据目录或 HOME 环境决定容量范围。
use crate::recovery_slot::SlotError;
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

pub(super) fn current_directory(deadline: Instant) -> Result<File, SlotError> {
    let home = current_home(deadline)?;
    open_directory(&home, deadline)
}

fn current_home(deadline: Instant) -> Result<PathBuf, SlotError> {
    check(deadline)?;
    let uid = unsafe { libc::geteuid() };
    if uid != unsafe { libc::getuid() } {
        return Err(SlotError::Unsupported);
    }
    let mut size = 1024;
    loop {
        check(deadline)?;
        let mut buffer = vec![0_u8; size];
        let mut password = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                password.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                size,
                &mut result,
            )
        };
        check(deadline)?;
        if code == libc::ERANGE && size < 64 << 10 {
            size *= 2;
            continue;
        }
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code).into());
        }
        if result.is_null() {
            return Err(SlotError::Unsupported);
        }
        let password = unsafe { password.assume_init() };
        if password.pw_dir.is_null() {
            return Err(SlotError::Unsupported);
        }
        let path = PathBuf::from(OsStr::from_bytes(
            unsafe { CStr::from_ptr(password.pw_dir) }.to_bytes(),
        ));
        if !path.is_absolute() {
            return Err(SlotError::Unsupported);
        }
        return Ok(path);
    }
}

// 仅供同模块测试提供真实隔离 home；生产定位始终由当前 OS 用户记录决定。
pub(super) fn open_directory(home: &Path, deadline: Instant) -> Result<File, SlotError> {
    check(deadline)?;
    if !home.is_absolute() {
        return Err(SlotError::Unsupported);
    }
    let mut directory = File::open("/")?;
    for component in home.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => directory = open_child(&directory, name, false, deadline)?,
            _ => return Err(SlotError::Unsupported),
        }
    }
    if directory.metadata()?.uid() != unsafe { libc::geteuid() } {
        return Err(SlotError::Unsupported);
    }
    for name in [".local", "state", "diskgraph", "recovery"] {
        directory = open_child(&directory, OsStr::new(name), true, deadline)?;
    }
    check(deadline)?;
    Ok(directory)
}

fn open_child(
    parent: &File,
    name: &OsStr,
    create: bool,
    deadline: Instant,
) -> Result<File, SlotError> {
    check(deadline)?;
    let name = CString::new(name.as_bytes()).map_err(|_| SlotError::Unsupported)?;
    if create {
        let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
        if created < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EEXIST) {
                return Err(error.into());
            }
        } else {
            parent.sync_all()?;
        }
    }
    check(deadline)?;
    let raw = unsafe {
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
    if raw < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(raw) };
    let metadata = file.metadata()?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir()
        || (metadata.uid() != 0 && metadata.uid() != uid)
        || metadata.mode() & 0o022 != 0
    {
        return Err(SlotError::Unsupported);
    }
    check(deadline)?;
    Ok(file)
}

fn check(deadline: Instant) -> Result<(), SlotError> {
    if Instant::now() >= deadline {
        Err(SlotError::Deadline)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "trusted_local_recovery_home_tests.rs"]
mod tests;
