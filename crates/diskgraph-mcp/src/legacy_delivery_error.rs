/// legacy 投递准入的明确拒绝原因，均在业务执行前映射为 HTTP 错误。
/// 来源：DiskGraph 原生 Rust legacy SSE；无 Java 对应对象。
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum LegacyDeliveryError {
    UnknownSession,
    PrincipalMismatch,
    ResponseLimit,
    Backpressure,
    Unavailable,
}
