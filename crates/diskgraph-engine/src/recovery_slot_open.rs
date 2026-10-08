use crate::recovery_slot::SlotError;
use std::ffi::CStr;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Instant;

/// 打开原目录中的固定恢复槽；存在时不走创建路径，首次创建使用原子排他准入。
/// 参数：directory 为已核验原目录，name 为协议固定单组件名称，deadline 为原启动期限。
/// 返回：原文件句柄或原系统错误；创建竞争最多重开一次，不修复或重建原目录。
pub(crate) fn open(directory: &File, name: &CStr, deadline: Instant) -> Result<File, SlotError> {
    let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    match attempt(directory, name, flags, deadline) {
        Ok(file) => return Ok(file),
        Err(SlotError::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => {}
        Err(error) => return Err(error),
    }
    // 只对首次不存在进行独占创建；其他进程已创建时不再次使用 O_CREAT 打开活动槽。
    match attempt(
        directory,
        name,
        flags | libc::O_CREAT | libc::O_EXCL,
        deadline,
    ) {
        Err(SlotError::Io(error)) if error.raw_os_error() == Some(libc::EEXIST) => {
            attempt(directory, name, flags, deadline)
        }
        result => result,
    }
}

fn attempt(
    directory: &File,
    name: &CStr,
    flags: i32,
    deadline: Instant,
) -> Result<File, SlotError> {
    if Instant::now() >= deadline {
        return Err(SlotError::Deadline);
    }
    // 原目录及固定名称在同步调用期间有效；独占创建不覆盖任何已有槽正文。
    let raw = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(raw) })
}
