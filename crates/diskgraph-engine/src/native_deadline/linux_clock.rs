use super::clock_sample::ClockSample;
use crate::EngineError;
use diskgraph_core::BusinessError;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
/// 参数：无；返回：真实 nsfs 时间域与 CLOCK_MONOTONIC 纳秒，跨域/异常拒绝。
pub(super) fn read() -> Result<ClockSample, EngineError> {
    let namespace = File::open("/proc/self/ns/time")?;
    let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(namespace.as_raw_fd(), fs.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { fs.assume_init() }.f_type != 0x6e736673 {
        return Err(BusinessError::Unsupported.into());
    }
    let original = namespace.metadata()?;
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, time.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let time = unsafe { time.assume_init() };
    let current = File::open("/proc/self/ns/time")?.metadata()?;
    if original.dev() != current.dev() || original.ino() != current.ino() {
        return Err(BusinessError::Conflict.into());
    }
    let seconds = u64::try_from(time.tv_sec).map_err(|_| BusinessError::Unsupported)?;
    let nanos = u64::try_from(time.tv_nsec).map_err(|_| BusinessError::Unsupported)?;
    if nanos >= 1_000_000_000 {
        return Err(BusinessError::Unsupported.into());
    }
    Ok(ClockSample {
        margin_nanos: 1,
        nanos: seconds
            .checked_mul(1_000_000_000)
            .and_then(|s| s.checked_add(nanos))
            .ok_or(BusinessError::Unsupported)?,
        domain: [1, original.dev(), original.ino()],
    })
}
