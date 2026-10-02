/// 批准校验查询的原始 SQLite tuple，保持摘要、主体、动作和期限列顺序。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
pub(crate) type ApprovalRow = (String, String, String, String, String, i64, i64, i64);
