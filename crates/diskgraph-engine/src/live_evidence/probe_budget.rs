use super::ProbeLimits;
use super::probe_failure::ProbeFailure;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// 每次采样独占的绝对期限、累计输出与中止水位。
/// 来源：原生 Rust diskgraph-engine::live_evidence::ProbeBudget。
pub(super) struct ProbeBudget {
    deadline: Instant,
    remaining: usize,
    cancel: Arc<AtomicBool>,
    failure: Option<ProbeFailure>,
}

impl ProbeBudget {
    /// 从配置建立唯一预算，后续子命令借用而不复制额度。
    /// 参数：limits 为调用方配置；上限超过 64 MiB 或期限不可表示时拒绝。
    /// 返回：已建立的预算或配置错误。
    pub(super) fn new(limits: &ProbeLimits) -> Result<Self, ProbeFailure> {
        let deadline = Instant::now()
            .checked_add(limits.timeout)
            .ok_or(ProbeFailure::InvalidLimits)?;
        if limits.max_output_bytes > 64 << 20 {
            return Err(ProbeFailure::InvalidLimits);
        }
        Ok(Self {
            deadline,
            remaining: limits.max_output_bytes,
            cancel: Arc::clone(&limits.cancel),
            failure: None,
        })
    }

    /// 每段工作前后复核整次采样的期限与取消，锁存首次失败。
    /// 参数：无。
    /// 返回：仍可继续，或本次采样首次中止原因。
    pub(super) fn check(&mut self) -> Result<(), ProbeFailure> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        if self.cancel.load(Ordering::Acquire) {
            return Err(self.fail(ProbeFailure::Cancelled));
        }
        if Instant::now() >= self.deadline {
            return Err(self.fail(ProbeFailure::Deadline));
        }
        Ok(())
    }

    /// 两条管道、所有子命令及 Git stash 日志在保留读取数据前扣除同一额度。
    /// 参数：bytes 为固定读取缓冲中实际观察的字节数。
    /// 返回：扣费成功，或累计超限；超读缓冲不得加入成功输出。
    pub(super) fn consume(&mut self, bytes: usize) -> Result<(), ProbeFailure> {
        self.check()?;
        let Some(remaining) = self.remaining.checked_sub(bytes) else {
            return Err(self.fail(ProbeFailure::OutputLimit));
        };
        self.remaining = remaining;
        Ok(())
    }

    /// 锁存执行或读取失败，保证后续命令不再启动。
    /// 参数：failure 为本次观察到的失败。
    /// 返回：首次失败；后来的失败不覆盖既有原因。
    pub(super) fn fail(&mut self, failure: ProbeFailure) -> ProbeFailure {
        self.failure.get_or_insert(failure).clone()
    }

    /// 读取真实剩余额度用于普通文件读取前准入及累计扣费回归。
    /// 参数：无。返回：尚未消费的输出及 stash 输入字节，不补充预算。
    pub(super) fn remaining_bytes(&self) -> usize {
        self.remaining
    }

    /// 将原绝对期限推进到已耗尽水位，不重建预算。参数：无。返回：无，仅供确定性期限回归。
    #[cfg(test)]
    pub(super) fn expire_for_test(&mut self) {
        self.deadline = Instant::now();
    }
}
