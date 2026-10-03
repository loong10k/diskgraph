//! adapter_status：既有文件操作职责的原生 Rust 实现。

/// 允许工具的可用版本或明确不可用原因。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::AdapterStatus`，保留既有语义。
/// Whether a specialist tool is usable, and if not, exactly why. An
/// unavailable adapter offers nothing: there is no degraded mode that
/// force-deletes directories instead (EC-03).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterStatus {
    Available { version: String },
    Unavailable { reason: String },
}
