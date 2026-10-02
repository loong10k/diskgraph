use std::sync::{Arc, Mutex};

use crate::legacy_delivery_error::LegacyDeliveryError;
use crate::legacy_delivery_state::LegacyDeliveryState;

/// 不可克隆的投递额度凭证；所有返回、断线及发送失败路径由 Drop 归还。
/// 来源：DiskGraph 原生 Rust legacy SSE；无 Java 对应对象。
pub(crate) struct LegacyReservation {
    pub(crate) state: Arc<Mutex<LegacyDeliveryState>>,
    pub(crate) session_id: String,
    pub(crate) bytes: usize,
}

impl LegacyReservation {
    /// 将最坏响应预留缩为实际帧字节。参数：bytes 是完整 SSE 帧长度；返回：缩减结果。
    pub(crate) fn shrink(&mut self, bytes: usize) -> Result<(), LegacyDeliveryError> {
        if bytes > self.bytes {
            return Err(LegacyDeliveryError::ResponseLimit);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| LegacyDeliveryError::Unavailable)?;
        let session = state
            .sessions
            .get_mut(&self.session_id)
            .ok_or(LegacyDeliveryError::UnknownSession)?;
        let released = self.bytes - bytes;
        session.reserved_bytes -= released;
        state.reserved_bytes -= released;
        self.bytes = bytes;
        Ok(())
    }
}

impl Drop for LegacyReservation {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.reserved_bytes -= self.bytes;
        if let Some(session) = state.sessions.get_mut(&self.session_id) {
            session.reserved_bytes -= self.bytes;
            session.reserved_messages -= 1;
        }
    }
}
