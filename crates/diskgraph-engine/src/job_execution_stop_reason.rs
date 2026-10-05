//! 本执行代次协作停止的原始错误所有权；来源：原生 Rust Engine 持久执行链。

use crate::EngineError;
use diskgraph_store::StoreError;
use std::sync::Mutex;

/// 保存 keeper 首个真实失败，供来源明确的协作停止分支移动取回。
/// 来源：DiskGraph 原生 Rust；不是授权、任务、连接或线程 owner，不复制 EngineError。
pub(super) struct JobExecutionStopReason {
    error: Mutex<Option<EngineError>>,
}

impl JobExecutionStopReason {
    /// 参数：无；返回：本次执行局部的空原因槽，不读取或更新持久状态。
    pub(super) fn new() -> Self {
        Self {
            error: Mutex::new(None),
        }
    }

    /// 参数：error 为 keeper 原真实失败；返回：无，保留首个错误而不覆盖为后续停止结果。
    /// 必须在设置本代停止 Atomic 之前调用；本锁只移动值，不调用授权、SQL 或原生 I/O。
    pub(super) fn record(&self, error: EngineError) {
        let mut slot = self
            .error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    /// 参数：无；返回：拥有的首个原因或无。仅来源明确的协作停止检查可以消费。
    pub(super) fn take(&self) -> Option<EngineError> {
        self.error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }

    /// 参数：outcome 为尚无已提交回执的实际结果；返回：仅还原固定纯提交停止错误的结果。
    /// 成功、独立身份 Conflict、其他存储/原生错误均保持原值；不能在提交事实保护之前调用。
    pub(super) fn restore_commit_stop(
        &self,
        outcome: Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        match outcome {
            Err(EngineError::Store(StoreError::Conflict(message)))
                if message == "scan cancelled before commit" =>
            {
                Err(self
                    .take()
                    .unwrap_or_else(|| StoreError::Conflict(message).into()))
            }
            other => other,
        }
    }

    /// 参数：callback 只分类并通知测试通道；返回：无，借用已经移动保存的实际错误。
    /// 来源：cfg(test) 阶段见证；回调不得等待或读取数据库，本次检查的控制锁已释放。
    #[cfg(test)]
    pub(super) fn observe(&self, callback: impl FnOnce(&EngineError)) {
        let slot = self
            .error
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(error) = slot.as_ref() {
            callback(error);
        }
    }
}
