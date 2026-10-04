//! 普通文件强历代身份与纳秒版本；来源：Linux name_to_handle_at(2)，只比较不透明句柄。
use super::linux_open::{filesystem, last_error, unique_mount};
use diskgraph_core::{
    IndexedFileEpoch, ProcessEvidenceFailureCode as Failure, UnixFileObservation,
};
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::time::{SystemTime, UNIX_EPOCH};

/// 参数：held 普通文件、原 boot 标识与原任务检查；返回：同一次 metadata 的版本及强 epoch。
pub(super) fn capture(
    file: &File,
    boot: &[u8; 36],
    check: &dyn Fn() -> Result<(), Failure>,
) -> Result<UnixFileObservation, Failure> {
    check()?;
    let start = now_ms()?;
    let before = file.metadata().map_err(|_| last_error())?;
    check()?;
    if !before.is_file() {
        return Err(Failure::Unsupported);
    }
    if !matches!(filesystem(file)?, 0xef53 | 0x01021994) {
        return Err(Failure::Unsupported);
    }
    check()?;
    let mount = unique_mount(file)?;
    check()?;
    let mut raw = [0_u64; 17];
    let handle = raw.as_mut_ptr().cast::<libc::file_handle>();
    unsafe {
        (*handle).handle_bytes = 128;
    }
    let mut ordinary_mount = 0;
    if unsafe {
        libc::name_to_handle_at(
            file.as_raw_fd(),
            c"".as_ptr(),
            handle,
            &mut ordinary_mount,
            libc::AT_EMPTY_PATH,
        )
    } != 0
    {
        return Err(last_error());
    }
    check()?;
    let (length, handle_type) = unsafe { ((*handle).handle_bytes as usize, (*handle).handle_type) };
    if length == 0 || length > 128 || handle_type < 0 {
        return Err(Failure::Unsupported);
    }
    let after = file.metadata().map_err(|_| last_error())?;
    check()?;
    if !same_version(&before, &after) {
        return Err(Failure::Conflict);
    }
    let mut h = Sha256::new();
    h.update(b"diskgraph-linux-file-boot-mount-v1\0");
    h.update(boot);
    h.update(mount.to_le_bytes());
    let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>().add(8), length) };
    UnixFileObservation::new(
        IndexedFileEpoch::LinuxHandle {
            device: before.dev(),
            inode: before.ino(),
            filesystem_domain_sha256: h.finalize().into(),
            handle_type,
            handle_bytes: bytes.to_vec(),
        },
        before.mode(),
        before.len(),
        (before.mtime(), before.mtime_nsec() as u32),
        (before.ctime(), before.ctime_nsec() as u32),
        (start, now_ms()?),
    )
    .map_err(|_| Failure::Unsupported)
}

/// 参数：两次同一 held 对象观察；返回：身份、长度、权限与纳秒版本全部相同。
pub(super) fn same_version(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.mode() == b.mode()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}

/// 参数：无；返回：有限真实 Unix 毫秒或时钟失败。
pub(super) fn now_ms() -> Result<u64, Failure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
        .ok_or(Failure::Unavailable)
}
