use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};

use crate::legacy_delivery_error::LegacyDeliveryError;
use crate::legacy_delivery_session::LegacyDeliverySession;
use crate::legacy_delivery_state::LegacyDeliveryState;
use crate::legacy_frame::LegacyFrame;
use crate::legacy_reservation::LegacyReservation;
use crate::legacy_session_receiver::LegacySessionReceiver;

pub(crate) const FRAME_OVERHEAD: usize = "event: message\ndata: \n\n".len();
const SESSION_BYTES: usize = 16 << 20;
const GLOBAL_BYTES: usize = 64 << 20;
const SESSION_MESSAGES: usize = 64;

/// 远程 legacy 独占的预算注册表，不暴露可绕过预留的裸 sender。
/// 来源：DiskGraph 原生 Rust legacy SSE；无 Java 对应对象。
#[derive(Clone)]
pub(crate) struct LegacyDeliveryRegistry {
    pub(crate) inner: Arc<Mutex<LegacyDeliveryState>>,
}

impl LegacyDeliveryRegistry {
    /// 创建监听器共享预算。参数：无额外输入；返回：每会话 16 MiB/64 条、全局 64 MiB 的注册表。
    pub(crate) fn new() -> Self {
        Self::with_limits(GLOBAL_BYTES, SESSION_BYTES, SESSION_MESSAGES)
    }

    fn with_limits(global_limit: usize, session_limit: usize, message_limit: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LegacyDeliveryState {
                sessions: HashMap::new(),
                reserved_bytes: 0,
                global_limit,
                session_limit,
                message_limit,
            })),
        }
    }

    /// 建立会话。参数：principal 为握手主体；返回：会话 ID 和自动关闭的接收端。
    pub(crate) fn open(
        &self,
        principal: Option<String>,
    ) -> Result<(String, LegacySessionReceiver), LegacyDeliveryError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| LegacyDeliveryError::Unavailable)?;
        let session_id = format!("legacy-{}", uuid::Uuid::new_v4());
        let (sender, receiver) = mpsc::sync_channel(state.message_limit);
        state.sessions.insert(
            session_id.clone(),
            LegacyDeliverySession {
                principal,
                sender,
                reserved_bytes: 0,
                reserved_messages: 0,
            },
        );
        Ok((
            session_id.clone(),
            LegacySessionReceiver {
                receiver,
                registry: self.clone(),
                session_id,
            },
        ))
    }

    /// 业务前认领额度。参数：session_id、principal、bytes 为会话、主体和最坏帧字节；返回：凭证或拒绝原因。
    pub(crate) fn reserve(
        &self,
        session_id: &str,
        principal: Option<&str>,
        bytes: usize,
    ) -> Result<LegacyReservation, LegacyDeliveryError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| LegacyDeliveryError::Unavailable)?;
        let session = state
            .sessions
            .get(session_id)
            .ok_or(LegacyDeliveryError::UnknownSession)?;
        if session.principal.as_deref() != principal {
            return Err(LegacyDeliveryError::PrincipalMismatch);
        }
        if bytes > state.session_limit || bytes > state.global_limit {
            return Err(LegacyDeliveryError::ResponseLimit);
        }
        if session.reserved_messages >= state.message_limit
            || bytes > state.session_limit.saturating_sub(session.reserved_bytes)
            || bytes > state.global_limit.saturating_sub(state.reserved_bytes)
        {
            return Err(LegacyDeliveryError::Backpressure);
        }
        state.reserved_bytes += bytes;
        let session = state
            .sessions
            .get_mut(session_id)
            .expect("checked session under lock");
        session.reserved_bytes += bytes;
        session.reserved_messages += 1;
        Ok(LegacyReservation {
            state: Arc::clone(&self.inner),
            session_id: session_id.to_owned(),
            bytes,
        })
    }

    /// 交接已编码结果。参数：frame 携带真实字节及凭证；返回：投递结果，错误自动归还。
    pub(crate) fn enqueue(&self, frame: LegacyFrame) -> Result<(), LegacyDeliveryError> {
        // 必须在锁外发送/销毁帧，凭证 Drop 也会获取计数锁。
        let sender = {
            let state = self
                .inner
                .lock()
                .map_err(|_| LegacyDeliveryError::Unavailable)?;
            state
                .sessions
                .get(&frame._reservation.session_id)
                .map(|session| session.sender.clone())
        }
        .ok_or(LegacyDeliveryError::UnknownSession)?;
        sender.try_send(frame).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => LegacyDeliveryError::Backpressure,
            mpsc::TrySendError::Disconnected(_) => LegacyDeliveryError::UnknownSession,
        })
    }

    /// 关闭会话。参数：session_id 为目标 ID；返回：无，移除 sender，旧凭证释放后才退全局额度。
    pub(crate) fn close(&self, session_id: &str) {
        let removed = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .sessions
            .remove(session_id);
        drop(removed);
    }
}

#[cfg(test)]
#[path = "legacy_budget_tests.rs"]
mod tests;
