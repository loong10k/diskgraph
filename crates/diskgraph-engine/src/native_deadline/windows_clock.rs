use super::clock_sample::ClockSample;
use crate::EngineError;
use diskgraph_core::BusinessError;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
/// 参数：无；返回：本机 QPC 整数纳秒及真实频率域，失败/溢出拒绝而不使用墙钟。
pub(super) fn read() -> Result<ClockSample, EngineError> {
    let mut frequency = 0;
    let mut counter = 0;
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0
        || unsafe { QueryPerformanceCounter(&mut counter) } == 0
    {
        return Err(BusinessError::Unsupported.into());
    }
    let frequency = u64::try_from(frequency).map_err(|_| BusinessError::Unsupported)?;
    let counter = u64::try_from(counter).map_err(|_| BusinessError::Unsupported)?;
    if frequency == 0 {
        return Err(BusinessError::Unsupported.into());
    }
    let nanos = u64::try_from(u128::from(counter) * 1_000_000_000 / u128::from(frequency))
        .map_err(|_| BusinessError::Unsupported)?;
    Ok(ClockSample {
        nanos,
        // QPC 跨线程/进程比较有一 tick 不确定性；另扣一纳秒整数换算余量。
        margin_nanos: 1_000_000_000u64
            .div_ceil(frequency)
            .checked_add(1)
            .ok_or(BusinessError::Unsupported)?,
        domain: [3, frequency, 0],
    })
}
