//! 共享图库写锁沿调用链原期限等待，不引入新连接或锁所有者。
use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;
use diskgraph_store::SqliteSnapshotStore;
use std::sync::{MutexGuard, TryLockError};
use std::time::{Duration, Instant};

impl Engine {
    /// 参数：deadline 为原请求单调截止时间；返回：同图库 guard 或预算/中毒错误。
    /// 与控制锁组合仍必须按 graph→control 顺序；迟到获取不能继续工作。
    pub(super) fn graph_until(
        &self,
        deadline: Instant,
    ) -> Result<MutexGuard<'_, SqliteSnapshotStore>, EngineError> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(BusinessError::BudgetExceeded.into());
            }
            match self.graph.try_lock() {
                Ok(guard) => {
                    if Instant::now() >= deadline {
                        return Err(BusinessError::BudgetExceeded.into());
                    }
                    return Ok(guard);
                }
                Err(TryLockError::Poisoned(_)) => return Err(EngineError::Poisoned),
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(remaining.min(Duration::from_millis(1)))
                }
            }
        }
    }
}
