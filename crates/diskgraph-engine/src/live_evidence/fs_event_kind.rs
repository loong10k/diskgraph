//! 区分轮询观察的出现、修改及移除事件。

/// 区分轮询观察的出现、修改及移除事件。
/// 来源：原生 Rust diskgraph-engine::live_evidence::FsEventKind。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsEventKind {
    Appeared,
    Modified,
    Removed,
}
