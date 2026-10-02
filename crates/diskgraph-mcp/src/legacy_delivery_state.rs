use std::collections::HashMap;

use crate::legacy_delivery_session::LegacyDeliverySession;

/// 同一监听器共享的投递计数，单锁保证全局和会话额度同时认领。
/// 来源：DiskGraph 原生 Rust legacy SSE；无 Java 对应对象。
pub(crate) struct LegacyDeliveryState {
    pub(crate) sessions: HashMap<String, LegacyDeliverySession>,
    pub(crate) reserved_bytes: usize,
    pub(crate) global_limit: usize,
    pub(crate) session_limit: usize,
    pub(crate) message_limit: usize,
}
