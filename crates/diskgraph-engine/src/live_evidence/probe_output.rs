/// 保存两管道完整输出和正常/信号退出状态；只有执行器成功才可解释。
/// 来源：原生 Rust diskgraph-engine::live_evidence::ProbeOutput。
#[derive(Debug)]
pub(super) struct ProbeOutput {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) exit_code: Option<i32>,
}

impl ProbeOutput {
    /// 在清理结束后裁决本次输出，保持整次采样的预算状态。
    /// 参数：budget 为原采样预算，cleanup 为原 owner 的清理结果。
    /// 返回：仍有效的完整输出，或带清理次因的失败。
    pub(super) fn finish(
        self,
        budget: &mut super::probe_budget::ProbeBudget,
        cleanup: Result<(), super::probe_failure::ProbeFailure>,
    ) -> Result<Self, super::probe_failure::ProbeFailure> {
        // 清理本身可能消耗期限或观察到取消；不能用次要清理错误遮蔽请求中止。
        match budget.check() {
            Ok(()) => self.with_cleanup(cleanup),
            Err(primary) => Err(primary.with_cleanup(cleanup)),
        }
    }

    /// 合并已经收齐的输出与原 owner 清理结果。
    /// 参数：cleanup 为实际执行后的清理状态；返回：完整输出或仍需恢复的失败。
    pub(super) fn with_cleanup(
        self,
        cleanup: Result<(), super::probe_failure::ProbeFailure>,
    ) -> Result<Self, super::probe_failure::ProbeFailure> {
        use super::probe_failure::ProbeFailure;
        match cleanup {
            Ok(()) => Ok(self),
            Err(cleanup) => match self.exit_code {
                Some(exit_code) if exit_code > 0 => Err(ProbeFailure::CommandExit {
                    exit_code,
                    stderr: std::sync::Arc::new(self.stderr),
                }
                .with_cleanup(Err(cleanup))),
                _ => Err(cleanup),
            },
        }
    }
}
