//! 原 pinned 枚举的封闭标签映射；来源：tree.rs / classify.rs。
use diskgraph_disktree_core::{
    classify::{Category, Reclaim},
    tree::NodeKind,
};
use std::io;

/// 参数：value 为真实 pinned NodeKind。
/// 返回：对应封闭线格式数字标签。
pub(crate) fn kind_tag(value: NodeKind) -> u8 {
    match value {
        NodeKind::Directory => 0,
        NodeKind::File => 1,
        NodeKind::Symlink => 2,
        NodeKind::Other => 3,
    }
}

/// 参数：value 为线格式节点类型标签。
/// 返回：真实 NodeKind；未知数字拒绝。
pub(crate) fn kind(value: u8) -> io::Result<NodeKind> {
    match value {
        0 => Ok(NodeKind::Directory),
        1 => Ok(NodeKind::File),
        2 => Ok(NodeKind::Symlink),
        3 => Ok(NodeKind::Other),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "unknown kind")),
    }
}

/// 参数：value 为真实 pinned Category。
/// 返回：保持九种类别区别的线格式标签。
pub(crate) fn category_tag(value: Category) -> u8 {
    match value {
        Category::Code => 0,
        Category::AgentScratch => 1,
        Category::Toolchain => 2,
        Category::Synced => 3,
        Category::Git => 4,
        Category::Media => 5,
        Category::Documents => 6,
        Category::Cache => 7,
        Category::Other => 8,
    }
}

/// 参数：value 为线格式类别数字。
/// 返回：真实 Category；未知标签不降级为 Other。
pub(crate) fn category(value: u8) -> io::Result<Category> {
    match value {
        0 => Ok(Category::Code),
        1 => Ok(Category::AgentScratch),
        2 => Ok(Category::Toolchain),
        3 => Ok(Category::Synced),
        4 => Ok(Category::Git),
        5 => Ok(Category::Media),
        6 => Ok(Category::Documents),
        7 => Ok(Category::Cache),
        8 => Ok(Category::Other),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown category",
        )),
    }
}

/// 参数：value 为真实 pinned Reclaim。
/// 返回：保持九种回收原因的线格式标签。
pub(crate) fn reclaim_tag(value: Reclaim) -> u8 {
    match value {
        Reclaim::Regenerable => 0,
        Reclaim::SyncHistory => 1,
        Reclaim::PackageStore => 2,
        Reclaim::BuildOutput => 3,
        Reclaim::Reinstallable => 4,
        Reclaim::SandboxLayers => 5,
        Reclaim::Snapshots => 6,
        Reclaim::Trash => 7,
        Reclaim::Temporary => 8,
    }
}

/// 参数：value 为线格式回收原因数字。
/// 返回：真实 Reclaim；未知标签返回错误。
pub(crate) fn reclaim(value: u8) -> io::Result<Reclaim> {
    match value {
        0 => Ok(Reclaim::Regenerable),
        1 => Ok(Reclaim::SyncHistory),
        2 => Ok(Reclaim::PackageStore),
        3 => Ok(Reclaim::BuildOutput),
        4 => Ok(Reclaim::Reinstallable),
        5 => Ok(Reclaim::SandboxLayers),
        6 => Ok(Reclaim::Snapshots),
        7 => Ok(Reclaim::Trash),
        8 => Ok(Reclaim::Temporary),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown reclaim",
        )),
    }
}
