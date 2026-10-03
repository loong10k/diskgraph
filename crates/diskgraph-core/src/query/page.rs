use crate::DiskNode;

/// 分页节点与后续偏移；来源：DiskGraph 原生 Rust query::Page。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<'a> {
    pub items: Vec<&'a DiskNode>,
    pub next_offset: Option<usize>,
}
