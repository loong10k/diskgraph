use super::{Change, Incompatibility};

/// 按精确定位对齐的历史变化结果；来源：DiskGraph 原生 Rust query::Changes。
/// Result of comparing two snapshots by exact lossless locator.
pub struct Changes<'a> {
    pub incompatible: Option<Incompatibility>,
    pub changes: Vec<Change<'a>>,
}
