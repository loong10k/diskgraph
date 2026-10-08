//! 基准进程生命周期内存高水位；失败与未观测不伪装为零。

/// 参数：无；返回：当前进程生命周期 RSS/工作集高水位（字节），测量失败为未知。
pub(crate) fn rss() -> Option<u64> {
    #[cfg(unix)]
    {
        unix_rss(libc::RUSAGE_SELF)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let bytes = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: bytes,
            ..Default::default()
        };
        // 当前进程伪句柄无需关闭；此统计是工作集而不是提交内存或进程树 RSS。
        let observed =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, bytes) };
        (observed != 0).then_some(counters.PeakWorkingSetSize as u64)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// 参数：无；返回：Unix 已回收子进程的独立高水位；无对应 Windows 观测则为未知。
pub(crate) fn child_rss() -> Option<u64> {
    #[cfg(unix)]
    {
        unix_rss(libc::RUSAGE_CHILDREN)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(unix)]
fn unix_rss(who: libc::c_int) -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(who, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    let raw = u64::try_from(unsafe { usage.assume_init() }.ru_maxrss).ok()?;
    // macOS 原值为字节，Linux 为 KiB；失败与整数溢出不能报告为有效观测。
    raw.checked_mul(if cfg!(target_os = "macos") { 1 } else { 1024 })
}

/// 参数：父/子独立高水位；返回：都已知且无溢出时的和，不表示同时峰值或严格内存上限。
pub(crate) fn combined_rss(parent: Option<u64>, children: Option<u64>) -> Option<u64> {
    parent?.checked_add(children?)
}

#[cfg(test)]
mod tests {
    use super::{child_rss, combined_rss, rss};

    #[test]
    fn unknown_or_overflow_is_not_a_measured_sum() {
        assert_eq!(combined_rss(Some(10), None), None);
        assert_eq!(combined_rss(None, Some(10)), None);
        assert_eq!(combined_rss(Some(u64::MAX), Some(1)), None);
        assert_eq!(combined_rss(Some(10), Some(20)), Some(30));
    }

    #[test]
    fn native_host_observes_positive_lifetime_memory() {
        assert!(rss().is_some_and(|bytes| bytes > 0));
        #[cfg(windows)]
        assert_eq!(child_rss(), None);
        #[cfg(unix)]
        let _ = child_rss();
    }
}
