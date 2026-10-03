//! docker_object：既有文件操作职责的原生 Rust 实现。

/// Docker 自身报告的确切对象标识、类别和字节数。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::docker::DockerObject`，保留既有语义。
/// One exact Docker object, ready for a human to review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerObject {
    /// The id Docker itself reports (image id, container id, volume name,
    /// build-cache id). Cleanup commands name this and nothing else.
    pub id: String,
    /// What the object is, in Docker's own words.
    pub kind: &'static str,
    pub detail: String,
    pub bytes: u64,
}
