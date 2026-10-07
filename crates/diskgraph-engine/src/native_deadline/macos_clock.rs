use super::clock_sample::ClockSample;
use crate::EngineError;
use diskgraph_core::BusinessError;
/// 参数：无；返回：与 Rust Instant 同源的 CLOCK_UPTIME_RAW，不使用可调 UTC。
pub(super) fn read() -> Result<ClockSample, EngineError> {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, time.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let time = unsafe { time.assume_init() };
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
        domain: [2, 0, 0],
    })
}
