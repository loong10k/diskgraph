//! 当前线程CPU观察，仅用于显式调试诊断；来源：Q-02，无Java对等对象。
use std::time::Duration;

/// 保留同一线程的起始CPU计数；未知不作零值，不能作为请求期限或资源预算。
/// 来源：原生Rust Q-02授权失败诊断。
pub(super) struct ThreadCpuObservation {
    started: Option<u128>,
}

impl ThreadCpuObservation {
    /// 参数：enabled为原诊断开关；返回：起始计数，关闭时不调用平台时钟。
    pub(super) fn start(enabled: bool) -> Self {
        Self {
            started: if enabled { read_cpu_nanos() } else { None },
        }
    }

    /// 参数：无，须由原线程调用；返回：CPU耗时，读取失败/倒退/溢出均为None。
    pub(super) fn elapsed(&self) -> Option<Duration> {
        let started = self.started?;
        elapsed_nanos(Some(started), read_cpu_nanos())
    }
}

fn elapsed_nanos(started: Option<u128>, ended: Option<u128>) -> Option<Duration> {
    let elapsed = ended?.checked_sub(started?)?;
    Some(Duration::from_nanos(u64::try_from(elapsed).ok()?))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_cpu_nanos() -> Option<u128> {
    let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // 只在系统调用成功后读取完整初始化的timespec，不采样进程或其他线程。
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, value.as_mut_ptr()) } != 0 {
        return None;
    }
    let value = unsafe { value.assume_init() };
    let seconds = u128::try_from(value.tv_sec).ok()?;
    let nanos = u128::try_from(value.tv_nsec).ok()?;
    if nanos >= 1_000_000_000 {
        return None;
    }
    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}

#[cfg(windows)]
fn read_cpu_nanos() -> Option<u128> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // 当前线程伪句柄仅借用，不拥有、不关闭；失败时不使用输出字段。
    if unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return None;
    }
    let ticks = |value: FILETIME| {
        (u128::from(value.dwHighDateTime) << 32) | u128::from(value.dwLowDateTime)
    };
    ticks(kernel).checked_add(ticks(user))?.checked_mul(100)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn read_cpu_nanos() -> Option<u128> {
    None
}

#[cfg(test)]
mod tests {
    use super::{ThreadCpuObservation, elapsed_nanos};
    use std::time::Duration;

    #[test]
    fn disabled_cpu_observation_remains_unknown() {
        assert_eq!(ThreadCpuObservation::start(false).elapsed(), None);
    }

    #[test]
    fn absent_reversed_and_overflowing_cpu_are_not_zero_measurements() {
        assert_eq!(elapsed_nanos(None, Some(3)), None);
        assert_eq!(elapsed_nanos(Some(3), None), None);
        assert_eq!(elapsed_nanos(Some(3), Some(2)), None);
        assert_eq!(elapsed_nanos(Some(0), Some(u128::MAX)), None);
        assert_eq!(elapsed_nanos(Some(3), Some(3)), Some(Duration::ZERO));
        assert_eq!(
            elapsed_nanos(Some(3), Some(10)),
            Some(Duration::from_nanos(7))
        );
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    fn native_cpu_observation_reads_the_same_current_thread() {
        let observation = ThreadCpuObservation::start(true);
        for value in 0..10000 {
            std::hint::black_box(value * 3);
        }
        assert!(
            observation.elapsed().is_some(),
            "native CPU observation unavailable"
        );
    }
}
