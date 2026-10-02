use crate::StoreError;

/// 以 StoreError 为错误类型的统一存储结果。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
pub type Result<T> = std::result::Result<T, StoreError>;
