//! 保存单侧核验的可用摘要和读取成本，失败也保留已读字节。

/// 保存单侧核验的可用摘要和读取成本，失败也保留已读字节。
/// 来源：原生 Rust diskgraph-engine::verify::OneDigest。
/// One side's digest, or nothing when it could not be read.
pub(super) struct OneDigest {
    pub(super) digest: Option<String>,
    pub(super) bytes_read: u64,
}
