use super::ChildSpawnError;
use super::linux_atomic_child::LinuxAtomicChild;

/// 同时保留启动主错、清理失败和原 wait 未消费时的唯一处置 owner，不降级数字信号。
/// 来源：原生 Rust 原子 pidfd 失败所有权与 ChildSpawnError 类型传播合同。
pub(crate) struct LinuxAtomicLaunchFailure<E> {
    error: ChildSpawnError<E>,
    owner: Option<LinuxAtomicChild>,
}

impl<E> LinuxAtomicLaunchFailure<E> {
    /// 参数：error 为出生前原错；返回：没有创建 child 的失败。
    pub(super) fn before_birth(error: ChildSpawnError<E>) -> Self {
        Self { error, owner: None }
    }

    /// 参数：error 为原主错，child 为真实唯一owner；返回：清理结果与必要的处置owner。
    pub(super) fn after_birth(error: ChildSpawnError<E>, mut child: LinuxAtomicChild) -> Self {
        match child.cleanup() {
            Ok(()) => Self { error, owner: None },
            Err(cleanup) => {
                // POLLIN/stopped 不是已消费 wait；即使已退出仍保留原 reap 处置责任。
                let reaped = child.reaped();
                Self {
                    error: error.with_cleanup(Err(cleanup)),
                    owner: (!reaped).then_some(child),
                }
            }
        }
    }

    /// 参数：self 为失败材料；返回：原错误及仍需宿主显式回收的唯一owner，不授予继续扫描许可。
    pub(crate) fn into_parts(self) -> (ChildSpawnError<E>, Option<LinuxAtomicChild>) {
        (self.error, self.owner)
    }
}

impl<E: std::fmt::Debug> std::fmt::Debug for LinuxAtomicLaunchFailure<E> {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("LinuxAtomicLaunchFailure")
            .field("error", &self.error)
            .field("retained_owner", &self.owner.is_some())
            .finish()
    }
}
