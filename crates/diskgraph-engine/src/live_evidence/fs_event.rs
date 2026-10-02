//! 记录轮询可观察的路径变化，重命名表现为移除与出现。

use super::FsEventKind;
use std::path::PathBuf;

/// 记录轮询可观察的路径变化，重命名表现为移除与出现。
/// 来源：原生 Rust diskgraph-engine::live_evidence::FsEvent。
/// One observed change in a watched tree. A rename under polling arrives as
/// a `Removed` plus an `Appeared` pair, because that is what the sampling
/// can honestly distinguish (EV-03).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FsEvent {
    pub path: PathBuf,
    pub kind: FsEventKind,
}
