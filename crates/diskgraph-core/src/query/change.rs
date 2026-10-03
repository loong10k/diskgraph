use crate::{DiskNode, ResourceLocator};

/// 可比较历史中的新增、移除或已知尺寸变化；来源：DiskGraph 原生 Rust query::Change。
/// One observed difference between two comparable snapshots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change<'a> {
    Added {
        node: &'a DiskNode,
    },
    Removed {
        locator: &'a ResourceLocator,
        subtree_bytes: u64,
        name: &'a str,
    },
    SizeChanged {
        node: &'a DiskNode,
        previous_bytes: u64,
    },
}
