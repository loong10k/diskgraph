//! Linux 已知 FUSE 挂载的读取拒绝边界；不签发通用 provider 不召回资格。
use crate::EngineError;
use diskgraph_core::BusinessError;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

/// 参数：已由 O_PATH 获取的原对象句柄；返回：FUSE 拒绝、查询失败保留 I/O 错误。
/// 仅识别已复现提供方，不以其他 filesystem magic 判定全部资源可安全物化。
pub(crate) fn refuse_fuse(file: &File) -> Result<(), EngineError> {
    let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // 原 fd 存活，输出缓冲正确对齐；只查询挂载属性，不申请数据访问。
    if unsafe { libc::fstatfs(file.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let filesystem = unsafe { filesystem.assume_init() };
    if filesystem.f_type == 0x6573_5546 {
        return Err(BusinessError::Unsupported.into());
    }
    Ok(())
}

/// 参数：原 O_PATH 文件句柄；返回：绑定同一 inode 的普通读取句柄或精确拒绝。
/// /proc/self/fd 重新打开原对象，避免检查之后再次按客户端路径解析不同挂载/对象。
pub(crate) fn open_bound(file: &File) -> Result<File, EngineError> {
    refuse_fuse(file)?;
    let original = file.metadata()?;
    if !original.is_file() {
        return Err(BusinessError::InvalidArgument.into());
    }
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    let current = opened.metadata()?;
    if current.dev() != original.dev() || current.ino() != original.ino() {
        return Err(BusinessError::Conflict.into());
    }
    Ok(opened)
}
