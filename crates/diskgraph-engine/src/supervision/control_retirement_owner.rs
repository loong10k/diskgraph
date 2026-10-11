use crate::{Engine, ExecutorResourceOwner, SupervisorOwner, SupervisorRecoveryError};
use std::sync::Arc;
use std::time::Instant;

/// 原控制通道绑定的本进程退休责任；来源：PF-06 guardian/executor职责分离，无Java对等对象。
/// executor变体只持资源，不能取得容量槽；兼容变体保留原可信本地监督行为。
pub(crate) enum ControlRetirementOwner {
    Supervisor(SupervisorOwner),
    Executor(ExecutorResourceOwner),
}

impl ControlRetirementOwner {
    /// 参数：无；返回：前台存活时原Engine引用，不克隆或重建资源。
    pub(crate) fn engine(&self) -> Option<&Arc<Engine>> {
        match self {
            Self::Supervisor(owner) => owner.engine(),
            Self::Executor(owner) => owner.engine(),
        }
    }

    /// 参数：deadline为本轮原有限期限；返回：原责任实际退休，错误和Pending保留owner。
    /// executor成功不清槽，跨进程guardian仍须确认原child wait。
    pub(crate) fn poll_retirement(
        &mut self,
        deadline: Instant,
    ) -> Result<bool, SupervisorRecoveryError> {
        match self {
            Self::Supervisor(owner) => owner.poll_retirement(deadline),
            Self::Executor(owner) => owner.poll_retirement(deadline).map_err(Into::into),
        }
    }
}
