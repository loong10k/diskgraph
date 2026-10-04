//! 目录层的展示数据和必要节点投影。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

use super::Entry;
#[cfg(test)]
use diskgraph_core::DiskNode;
use diskgraph_store::NavigationNode;

/// 按需加载的一层目录展示数据；仅保留绘图和导航需要的字段。
/// 来源：DiskGraph 原生 Rust tui::Layer；无 Java 对应对象。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    pub parent_id: u64,
    pub offset: u64,
    pub has_more: bool,
    pub name: String,
    pub total_bytes: u64,
    pub total_files: u64,
    pub unreadable: bool,
    pub children: Vec<Entry>,
}

/// 将已读取节点投影为展示层，丢弃完整定位等不参与绘图的数据。
#[cfg(test)]
pub(crate) fn layer_from_nodes(
    node: DiskNode,
    children: Vec<DiskNode>,
    offset: u64,
    has_more: bool,
) -> Layer {
    Layer {
        parent_id: node.id,
        offset,
        has_more,
        name: node.name,
        total_bytes: node.subtree_bytes,
        total_files: node.files,
        unreadable: node.read_error,
        children: children
            .into_iter()
            .map(|child| Entry {
                id: child.id,
                name: child.name,
                size_bytes: child.subtree_bytes,
                files: child.files,
                kind: format!("{:?}", child.kind).to_lowercase(),
                category: child.category_hint,
                has_children: child.directories > 0,
                read_error: child.read_error,
            })
            .collect(),
    }
}

/// 将已经借用准入的必要节点字段映射到稳定导航层。
/// 参数：node/children 为真实窄投影；offset/has_more 保留显式分页语义。
/// 返回：原尺寸、分类、未知观测和类型标签语义的展示层。
pub(crate) fn layer_from_navigation_nodes(
    node: NavigationNode,
    children: Vec<NavigationNode>,
    offset: u64,
    has_more: bool,
) -> Layer {
    Layer {
        parent_id: node.id,
        offset,
        has_more,
        name: node.name,
        total_bytes: node.subtree_bytes,
        total_files: node.files,
        unreadable: node.read_error,
        children: children
            .into_iter()
            .map(|child| Entry {
                id: child.id,
                name: child.name,
                size_bytes: child.subtree_bytes,
                files: child.files,
                kind: format!("{:?}", child.kind).to_lowercase(),
                category: child.category_hint,
                has_children: child.directories > 0,
                read_error: child.read_error,
            })
            .collect(),
    }
}
