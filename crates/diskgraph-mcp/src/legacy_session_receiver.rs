use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::legacy_delivery_registry::LegacyDeliveryRegistry;
use crate::legacy_frame::LegacyFrame;

/// SSE 接收端的关闭守卫，结束时移除会话并释放队列中的所有帧预留。
/// 来源：DiskGraph 原生 Rust legacy SSE；无 Java 对应对象。
pub(crate) struct LegacySessionReceiver {
    pub(crate) receiver: Receiver<LegacyFrame>,
    pub(crate) registry: LegacyDeliveryRegistry,
    pub(crate) session_id: String,
}

impl LegacySessionReceiver {
    /// 等待一帧。参数：timeout 为等待期限；返回：帧、超时或关闭错误。
    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<LegacyFrame, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
}

impl Drop for LegacySessionReceiver {
    fn drop(&mut self) {
        self.registry.close(&self.session_id);
    }
}
