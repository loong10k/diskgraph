use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// 单个Windows根租约的有限诊断计数；来源：原生扫描根链校验成本定位。
/// 只计正常返回（含错误）的累计时间；不保存路径、不参与授权或执行期限。
#[derive(Default)]
pub(crate) struct WindowsScanCost {
    calls: AtomicU64,
    elapsed_ns: AtomicU64,
}

impl WindowsScanCost {
    /// 参数：work 为原同步根校验；返回：原结果，不处理或替换原panic。
    pub(crate) fn measure<T>(&self, work: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = work();
        let elapsed = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        // 诊断饱和而不溢出；不获取会影响原错误传播的锁。
        add_saturating(&self.elapsed_ns, elapsed);
        add_saturating(&self.calls, 1);
        result
    }

    /// 参数：无；返回：已返回调用数和累计毫秒，不代表CPU、文件I/O或RSS。
    /// 用于扫描阶段结束后的采样；并发采样不是跨字段原子快照。
    pub(crate) fn snapshot(&self) -> (u64, u64) {
        (
            self.calls.load(Ordering::Relaxed),
            self.elapsed_ns.load(Ordering::Relaxed) / 1_000_000,
        )
    }
}

fn add_saturating(counter: &AtomicU64, value: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    // 使用跨工具链稳定的CAS；竞争失败后基于最新值重算，避免丢失增量。
    loop {
        match counter.compare_exchange_weak(
            current,
            current.saturating_add(value),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowsScanCost, add_saturating};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn records_completed_original_results_without_substitution() {
        let cost = WindowsScanCost::default();
        let original: Result<(), u8> = cost.measure(|| Err(17));
        assert_eq!(original, Err(17));
        assert_eq!(cost.measure(|| 29), 29);
        assert_eq!(cost.snapshot().0, 2);
    }

    #[test]
    fn original_panic_payload_propagates_without_recording_completion() {
        let cost = WindowsScanCost::default();
        let result = std::panic::catch_unwind(|| cost.measure(|| std::panic::panic_any(37_u8)));
        assert_eq!(*result.unwrap_err().downcast::<u8>().unwrap(), 37);
        assert_eq!(cost.snapshot(), (0, 0));
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let counter = AtomicU64::new(u64::MAX - 1);
        add_saturating(&counter, 5);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        add_saturating(&counter, 1);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn concurrent_increments_preserve_all_completed_calls() {
        let counter = AtomicU64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..10_000 {
                        add_saturating(&counter, 1);
                    }
                });
            }
        });
        assert_eq!(counter.load(Ordering::Relaxed), 80_000);
    }
}
