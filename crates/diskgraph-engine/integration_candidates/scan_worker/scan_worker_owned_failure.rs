use crate::scan_worker_child::ScanWorkerChild;
use crate::scan_worker_failure::ScanWorkerFailure;

/// 保留原失败及未完成处置的唯一 child；领取此材料不授予扫描或发布。
/// 来源：原生 Rust PF-06 失败 owner 显式处置合同，无 Java 对等对象。
pub(crate) struct ScanWorkerOwnedFailure<E> {
    failure: ScanWorkerFailure<E>,
    owner: Option<ScanWorkerChild>,
}

impl<E> ScanWorkerOwnedFailure<E> {
    /// 参数：failure 为原主错误，owner 为唯一 child；返回：一次真实处置后的失败材料。
    /// 清理拒绝时保守移交原 owner；不以停止/EOF 替代原 wait，不覆盖原错误对象。
    pub(super) fn dispose(failure: ScanWorkerFailure<E>, mut owner: ScanWorkerChild) -> Self {
        match owner.cleanup() {
            Ok(()) => Self {
                failure,
                owner: None,
            },
            Err(cleanup) => Self {
                failure: failure.with_cleanup(Err(cleanup)),
                owner: Some(owner),
            },
        }
    }

    /// 参数：消费本失败材料；返回：原失败及仍需宿主显式处置的唯一 owner。
    /// 宿主不得仅取错误后丢弃 owner；未完成回收仍占用活动进程容量。
    pub(crate) fn into_parts(self) -> (ScanWorkerFailure<E>, Option<ScanWorkerChild>) {
        (self.failure, self.owner)
    }
}

impl<E: std::fmt::Debug> std::fmt::Debug for ScanWorkerOwnedFailure<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScanWorkerOwnedFailure")
            .field("failure", &self.failure)
            .field("retained_owner", &self.owner.is_some())
            .finish()
    }
}
