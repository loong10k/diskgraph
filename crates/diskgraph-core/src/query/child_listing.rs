use crate::DiskNode;

/// 子项分页结果及未纳入尺寸排序的未知项计数；来源：DiskGraph 原生 Rust query::ChildListing。
/// How a child listing ended, including honest coverage reporting.
pub struct ChildListing<'a> {
    pub items: Vec<&'a DiskNode>,
    pub next_offset: Option<usize>,
    /// Nodes whose size could not be reported, kept out of the ordering.
    pub unknown_count: usize,
    pub truncated: bool,
}
