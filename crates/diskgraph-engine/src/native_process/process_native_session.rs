use super::handle_reservation::HandleReservation;
use diskgraph_core::{ProcessEvidenceFailureCode as Failure, ProcessEvidenceLimits};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 复用原认领时钟与权限检查的原生观察账本；来源：Rust D42，不拥有任务状态或新线程。
/// 所有采样共用此实例；失败锁存，原生同步调用只在边界协作检查，不声称硬实时中断。
pub struct ProcessNativeSession<'a> {
    limits: &'a ProcessEvidenceLimits,
    started: Instant,
    cancel: &'a AtomicBool,
    check_authority: &'a dyn Fn() -> Result<(), Failure>,
    failure: Cell<Option<Failure>>,
    raw: Cell<u64>,
    entries: Cell<u64>,
    allocation: Cell<u64>,
    result: Cell<u64>,
    handles: Cell<u32>,
}
impl<'a> ProcessNativeSession<'a> {
    /// 参数：固定服务限额、原认领时刻、取消及原任务权限/fence 回调；返回：新任务唯一原生账本。
    pub fn new(
        limits: &'a ProcessEvidenceLimits,
        started: Instant,
        cancel: &'a AtomicBool,
        check_authority: &'a dyn Fn() -> Result<(), Failure>,
    ) -> Result<Self, Failure> {
        limits.validate().map_err(|_| Failure::InternalError)?;
        let value = Self {
            limits,
            started,
            cancel,
            check_authority,
            failure: Cell::new(None),
            raw: Cell::new(0),
            entries: Cell::new(0),
            allocation: Cell::new(0),
            result: Cell::new(0),
            handles: Cell::new(0),
        };
        value.check()?;
        Ok(value)
    }
    /// 参数：无；返回：原期限/取消/权限/fence 仍有效；首个失败不会被后续调用清除。
    pub fn check(&self) -> Result<(), Failure> {
        if let Some(error) = self.failure.get() {
            return Err(error);
        }
        let result = if self.cancel.load(Ordering::SeqCst) {
            Err(Failure::Cancelled)
        } else if self.started.elapsed() >= Duration::from_millis(self.limits.max_duration_ms()) {
            Err(Failure::Timeout)
        } else {
            // 同步授权回调可能耗时或触发取消；保留其原错误，成功后纯读同一时钟/标志。
            (self.check_authority)().and_then(|()| {
                if self.cancel.load(Ordering::SeqCst) {
                    Err(Failure::Cancelled)
                } else if self.started.elapsed()
                    >= Duration::from_millis(self.limits.max_duration_ms())
                {
                    Err(Failure::Timeout)
                } else {
                    Ok(())
                }
            })
        };
        if let Err(error) = result {
            self.failure.set(Some(error));
        }
        result
    }
    /// 参数：即将读取的最大元数据、条目及拥有分配字节；返回：读取/分配前累计准入。
    pub fn admit(&self, raw: u64, entries: u64, allocation: u64) -> Result<(), Failure> {
        self.check()?;
        let r = self.raw.get().checked_add(raw);
        let e = self.entries.get().checked_add(entries);
        let a = self.allocation.get().checked_add(allocation);
        if r.is_none_or(|v| v > self.limits.max_metadata_bytes())
            || e.is_none_or(|v| v > self.limits.max_entries())
            || a.is_none_or(|v| v > self.limits.max_allocation_bytes())
        {
            return self.fail(Failure::BudgetExceeded);
        }
        self.raw.set(r.expect("admitted"));
        self.entries.set(e.expect("admitted"));
        self.allocation.set(a.expect("admitted"));
        Ok(())
    }
    /// 参数：有界结果所需字节；返回：同一会话累计结果准入，不能重置额度。
    pub fn admit_result(&self, bytes: u64) -> Result<(), Failure> {
        self.check()?;
        let value = self.result.get().checked_add(bytes);
        if value.is_none_or(|v| v > self.limits.max_result_bytes()) {
            return self.fail(Failure::BudgetExceeded);
        }
        self.result.set(value.expect("admitted"));
        Ok(())
    }
    /// 参数：固定同时持有数及同步工作；返回：句柄边界内工作结果，退出释放额度。
    pub fn with_handles<T>(
        &self,
        count: u32,
        work: impl FnOnce() -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        self.check()?;
        let next = self.handles.get().checked_add(count);
        if next.is_none_or(|v| v > self.limits.max_handles()) {
            return self.fail(Failure::BudgetExceeded);
        }
        self.handles.set(next.expect("admitted"));
        let reservation = HandleReservation::new(&self.handles, count);
        let result = work();
        drop(reservation);
        match result {
            Ok(value) => {
                self.check()?;
                Ok(value)
            }
            Err(error) => self.fail(error),
        }
    }
    /// 参数：无；返回：实际累计元数据、条目、分配与结果额度，非 SQLite C/RSS 计量。
    pub fn usage(&self) -> (u64, u64, u64, u64) {
        (
            self.raw.get(),
            self.entries.get(),
            self.allocation.get(),
            self.result.get(),
        )
    }
    /// 参数：真实固定失败类别；返回：锁存该类别并停止此会话。
    pub fn fail<T>(&self, error: Failure) -> Result<T, Failure> {
        let error = self.failure.get().unwrap_or(error);
        self.failure.set(Some(error));
        Err(error)
    }
}
