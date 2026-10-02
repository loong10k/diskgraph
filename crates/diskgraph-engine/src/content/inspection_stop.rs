//! 解释内容检查未开始或未完成的原因；中止摘要不能作为确认结果。

/// 解释内容检查未开始或未完成的原因；中止摘要不能作为确认结果。
/// 来源：原生 Rust diskgraph-engine::content::InspectionStop。
/// Why a read or digest did not happen or did not finish.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectionStop {
    /// The object is a cloud placeholder and the policy forbids hydration.
    Placeholder,
    /// The caller asked to cancel.
    Cancelled,
    /// The object changed while being read: the result is void.
    Unstable,
    /// 字节预算不足以确认完整内容。
    ByteLimit,
    /// 总核验期限已到，后续摘要不可作为确认结果。
    Deadline,
    /// 已读内容后发生撤权，读取成本仍须保留。
    PermissionRevoked,
    /// 读取中断或 I/O 错误，部分摘要作废。
    ReadError,
}
