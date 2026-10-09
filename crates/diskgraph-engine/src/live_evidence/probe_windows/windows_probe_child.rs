use crate::live_evidence::probe_budget::ProbeBudget;
use crate::live_evidence::probe_failure::ProbeFailure;
use crate::native_child::WindowsChild;
#[cfg(test)]
use std::process::Command;

#[cfg(test)]
thread_local! {
    pub(super) static NATIVE_ACTIVITY_WAITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// 原探针的薄入口，只借原账本并保留原业务错误。来源：原生 Rust diskgraph-engine::WindowsProbeChild。
pub(in crate::live_evidence) struct WindowsProbeChild {
    child: WindowsChild,
}

impl WindowsProbeChild {
    /// 参数：child为catch外保留的原生唯一owner；返回：薄适配，不创建或清理进程。
    pub(in crate::live_evidence) fn from_child(child: WindowsChild) -> Self {
        Self { child }
    }

    /// 参数：消费薄适配；返回：同一原生owner，错误投影前交还显式恢复边界。
    pub(in crate::live_evidence) fn into_child(self) -> WindowsChild {
        self.child
    }

    /// 借用整次探针预算启动真实子进程。参数：command 为原命令，budget 为原账本；返回：唯一 OS owner 的薄入口或原探针错误。
    #[cfg(test)]
    pub(in crate::live_evidence) fn spawn(
        command: &mut Command,
        budget: &mut ProbeBudget,
    ) -> Result<Self, ProbeFailure> {
        let child = crate::native_child::WindowsTestBirth::spawn(command, || budget.check())
            .map_err(ProbeFailure::from)?;
        Ok(Self { child })
    }

    /// 观察但保留原 leader 身份。参数：无；返回：原操作结果或原探针错误。
    pub(in crate::live_evidence) fn poll(&mut self) -> Result<bool, ProbeFailure> {
        self.child.poll().map_err(ProbeFailure::from)
    }

    /// 在原期限及至多 5ms 内等待原生 I/O。参数：budget 为同一次采样账本；返回：唤醒或原失败。
    /// 唤醒不授予输出、EOF 或退出许可；不续期，前后仍检查原取消与期限。
    pub(in crate::live_evidence) fn wait_for_activity(
        &self,
        budget: &mut ProbeBudget,
    ) -> Result<(), ProbeFailure> {
        budget.check()?;
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_millis(5))
            .map_or(budget.deadline(), |tick| tick.min(budget.deadline()));
        self.child
            .wait_for_io_until(deadline)
            .map_err(ProbeFailure::from)?;
        #[cfg(test)]
        NATIVE_ACTIVITY_WAITS.with(|count| count.set(count.get().saturating_add(1)));
        budget.check()
    }

    /// 读取固定 stdout 片段。参数：无；返回：原操作结果或原探针错误。
    pub(in crate::live_evidence) fn read_stdout(&mut self) -> Result<Option<&[u8]>, ProbeFailure> {
        self.child.read_stdout().map_err(ProbeFailure::from)
    }

    /// 读取固定 stderr 片段。参数：无；返回：原操作结果或原探针错误。
    pub(in crate::live_evidence) fn read_stderr(&mut self) -> Result<Option<&[u8]>, ProbeFailure> {
        self.child.read_stderr().map_err(ProbeFailure::from)
    }

    /// 终止自有组或 Job 并实际回收。参数：无；返回：原操作结果或原探针错误。
    #[cfg(test)]
    pub(in crate::live_evidence) fn cleanup(&mut self) -> Result<(), ProbeFailure> {
        self.child.cleanup().map_err(ProbeFailure::from)
    }

    /// 读取stdout 实际 EOF。参数：无；返回：共用 owner 的原观察值。
    pub(in crate::live_evidence) fn stdout_eof(&self) -> bool {
        self.child.stdout_eof()
    }

    /// 读取stderr 实际 EOF。参数：无；返回：共用 owner 的原观察值。
    pub(in crate::live_evidence) fn stderr_eof(&self) -> bool {
        self.child.stderr_eof()
    }

    /// 读取实际观察的退出码。参数：无；返回：共用 owner 的原观察值。
    pub(in crate::live_evidence) fn exit_code(&self) -> Option<i32> {
        self.child.exit_code()
    }

    /// 复制原生验收句柄。参数：无；返回：测试独占句柄或原探针错误。
    #[cfg(test)]
    pub(super) fn duplicate_job_for_test(
        &self,
    ) -> Result<crate::native_child::OwnedHandle, ProbeFailure> {
        self.child
            .duplicate_job_for_test()
            .map_err(ProbeFailure::from)
    }

    /// 复制原生验收句柄。参数：无；返回：测试独占句柄或原探针错误。
    #[cfg(test)]
    pub(super) fn duplicate_leader_for_test(
        &self,
    ) -> Result<crate::native_child::OwnedHandle, ProbeFailure> {
        self.child
            .duplicate_leader_for_test()
            .map_err(ProbeFailure::from)
    }
}
