use std::sync::{MutexGuard, TryLockError};

use diskgraph_store::ControlStore;

use crate::{Engine, EngineError};

impl Engine {
    /// 可信宿主非阻塞获取控制库，用于有截止期限的内部授权检查。
    /// 来源：DiskGraph 原生 Rust Engine；无 Java 对应方法。
    /// 参数：无额外输入。
    /// 返回：可用 guard、锁繁忙的 None，或中毒错误；不等待锁。
    /// 非阻塞尝试获得既有控制库锁。
    /// 参数：无；供有期限的可信投递路径使用。
    /// 返回：可选同库 guard；竞争不阻塞，锁中毒返回错误。
    pub fn try_control_store(&self) -> Result<Option<MutexGuard<'_, ControlStore>>, EngineError> {
        match self.control.try_lock() {
            Ok(guard) => Ok(Some(guard)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Poisoned(_)) => Err(EngineError::Poisoned),
        }
    }
}

impl Engine {
    /// 在原请求期限内等待唯一控制连接，不把短暂竞争当作预算耗尽。
    /// 来源：DiskGraph 原生 Rust 状态查询；无 Java 对应方法。
    /// 参数：deadline 为调用链原始单调截止时间，不在此重建。
    /// 返回：同库 guard；到期返回预算错误，中毒保留原错误。
    pub(crate) fn control_until(
        &self,
        deadline: std::time::Instant,
    ) -> Result<MutexGuard<'_, ControlStore>, EngineError> {
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
            }
            if let Some(guard) = self.try_control_store()? {
                // 锁获取后仍复验，迟到获取不能成为有效请求。
                if std::time::Instant::now() >= deadline {
                    return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
                }
                return Ok(guard);
            }
            std::thread::sleep(remaining.min(std::time::Duration::from_millis(1)));
        }
    }
}
