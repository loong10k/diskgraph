use crate::live_evidence::probe_budget::ProbeBudget;
use crate::live_evidence::probe_failure::ProbeFailure;
use crate::native_child::UnixChild;
use std::process::Command;

/// 原探针的薄入口，只借原账本并保留原业务错误。来源：原生 Rust diskgraph-engine::UnixProbeChild。
pub(super) struct UnixProbeChild {
    child: UnixChild,
}

impl UnixProbeChild {
    /// 借用整次探针预算启动真实子进程。参数：command 为原命令，budget 为原账本；返回：唯一 OS owner 的薄入口或原探针错误。
    pub(super) fn spawn(
        command: &mut Command,
        budget: &mut ProbeBudget,
    ) -> Result<Self, ProbeFailure> {
        let child =
            UnixChild::spawn_checked(command, || budget.check()).map_err(ProbeFailure::from)?;
        Ok(Self { child })
    }

    /// 观察但保留原 leader 身份。参数：无；返回：原操作结果或原探针错误。
    pub(super) fn poll(&mut self) -> Result<bool, ProbeFailure> {
        self.child.poll().map_err(ProbeFailure::from)
    }

    /// 读取固定 stdout 片段。参数：无；返回：原操作结果或原探针错误。
    pub(super) fn read_stdout(&mut self) -> Result<Option<&[u8]>, ProbeFailure> {
        self.child.read_stdout().map_err(ProbeFailure::from)
    }

    /// 读取固定 stderr 片段。参数：无；返回：原操作结果或原探针错误。
    pub(super) fn read_stderr(&mut self) -> Result<Option<&[u8]>, ProbeFailure> {
        self.child.read_stderr().map_err(ProbeFailure::from)
    }

    /// 终止自有组或 Job 并实际回收。参数：无；返回：原操作结果或原探针错误。
    pub(super) fn cleanup(&mut self) -> Result<(), ProbeFailure> {
        self.child.cleanup().map_err(ProbeFailure::from)
    }

    /// 读取stdout 实际 EOF。参数：无；返回：共用 owner 的原观察值。
    pub(super) fn stdout_eof(&self) -> bool {
        self.child.stdout_eof()
    }

    /// 读取stderr 实际 EOF。参数：无；返回：共用 owner 的原观察值。
    pub(super) fn stderr_eof(&self) -> bool {
        self.child.stderr_eof()
    }

    /// 读取实际观察的退出码。参数：无；返回：共用 owner 的原观察值。
    pub(super) fn exit_code(&self) -> Option<i32> {
        self.child.exit_code()
    }
}
