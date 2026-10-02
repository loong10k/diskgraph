use std::sync::mpsc::SyncSender;

use crate::legacy_frame::LegacyFrame;

/// 会话主体、受限通道与未释放预留；计数包含发送中及尚未完成的业务。
/// 来源：DiskGraph 原生 Rust legacy SSE；无 Java 对应对象。
pub(crate) struct LegacyDeliverySession {
    pub(crate) principal: Option<String>,
    pub(crate) sender: SyncSender<LegacyFrame>,
    pub(crate) reserved_bytes: usize,
    pub(crate) reserved_messages: usize,
}
