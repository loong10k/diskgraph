//! Linux 元数据句柄的原生边界；来源：openat2(2)、statx(2)，不回退正文打开。
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::ffi::CStr;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};

// Linux UAPI 的 open_how 是三个 u64；数组保持原生 ABI，不依赖 libc 新版包装。
/// 参数：锚定目录、原生路径、打开标志及解析约束；返回：受约束句柄或固定错误。
pub(super) fn open_at(parent: i32, name: &CStr, flags: i32, resolve: u64) -> Result<File, Failure> {
    open_at_io(parent, name, flags, resolve).map_err(|error| error_code(error.raw_os_error()))
}

/// 参数：原锚、名称和原约束；返回：真实句柄或调用现场取得的原生错误。
pub(super) fn open_at_io(
    parent: i32,
    name: &CStr,
    flags: i32,
    resolve: u64,
) -> std::io::Result<File> {
    let how = [flags as u64, 0_u64, resolve];
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent,
            name.as_ptr(),
            how.as_ptr(),
            std::mem::size_of_val(&how),
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd as i32) })
}

/// 参数：已持有句柄；返回：真实内核文件系统类型，不猜测路径或设备名。
pub(super) fn filesystem(file: &File) -> Result<i128, Failure> {
    let mut value: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(file.as_raw_fd(), &mut value) } != 0 {
        return Err(last_error());
    }
    Ok(i128::from(value.f_type))
}

/// 参数：已持有句柄；返回：同一 boot 内不复用的挂载 ID，缺内核能力明确拒绝。
pub(super) fn unique_mount(file: &File) -> Result<u64, Failure> {
    let mut value: libc::statx = unsafe { std::mem::zeroed() };
    const UNIQUE: u32 = 0x4000;
    // statx 的 glibc 包装晚于 2.17；直接调用内核，保留 errno 与能力缺失拒绝。
    if unsafe {
        libc::syscall(
            libc::SYS_statx,
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_NO_AUTOMOUNT,
            UNIQUE,
            &mut value,
        )
    } != 0
    {
        return Err(last_error());
    }
    if value.stx_mask & UNIQUE == 0 || value.stx_mnt_id == 0 {
        return Err(Failure::Unsupported);
    }
    Ok(value.stx_mnt_id)
}

/// 参数：无；返回：最近原生失败的固定分类，不持久化路径或 errno 文本。
pub(super) fn last_error() -> Failure {
    error_code(std::io::Error::last_os_error().raw_os_error())
}

/// 参数：现场捕获的 errno；返回：固定原生失败分类。
pub(super) fn error_code(errno: Option<i32>) -> Failure {
    match errno {
        Some(libc::EACCES | libc::EPERM) => Failure::PermissionDenied,
        Some(libc::ENOENT | libc::ESRCH | libc::ESTALE) => Failure::Conflict,
        Some(
            libc::ENOSYS
            | libc::EOPNOTSUPP
            | libc::EINVAL
            | libc::EXDEV
            | libc::ELOOP
            | libc::EAGAIN,
        ) => Failure::Unsupported,
        Some(libc::EMFILE | libc::ENFILE | libc::ENOMEM | libc::EOVERFLOW) => {
            Failure::BudgetExceeded
        }
        _ => Failure::Unavailable,
    }
}

/// 参数：无符号编号和调用栈缓冲；返回：无分配的原生十进制名称。
pub(super) fn numeric_name(number: u32, buffer: &mut [u8; 12]) -> &CStr {
    let mut value = number;
    let mut offset = 10;
    buffer[11] = 0;
    loop {
        buffer[offset] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
        offset -= 1;
    }
    CStr::from_bytes_with_nul(&buffer[offset..]).expect("decimal digits and one NUL")
}
