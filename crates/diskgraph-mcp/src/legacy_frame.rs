use crate::legacy_reservation::LegacyReservation;

/// 已编码的 SSE 帧及额度生命周期，授权变更计数用于拒绝撤权前产生的积压结果。
/// 来源：DiskGraph 原生 Rust legacy SSE；无 Java 对应对象。
pub(crate) struct LegacyFrame {
    pub(crate) wire: String,
    pub(crate) authorization_generation: u64,
    // 帧内存先释放，随后额度凭证再释放，避免提前允许下一次分配。
    pub(crate) _reservation: LegacyReservation,
}
