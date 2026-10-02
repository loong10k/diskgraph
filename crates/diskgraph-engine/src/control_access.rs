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
