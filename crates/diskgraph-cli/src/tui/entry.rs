//! 单个已索引子节点的展示数据。
//! 来源：DiskGraph 原生 Rust TUI / OpenSpec Q-08；无 Java 对应实现。

/// 地图中的一个子节点，保留大小、分类、类型和读取错误状态。
/// 来源：DiskGraph 原生 Rust tui::Entry；无 Java 对应对象。
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub name: String,
    pub size_bytes: u64,
    pub files: u64,
    pub kind: String,
    pub category: Option<String>,
    pub has_children: bool,
    pub read_error: bool,
}
