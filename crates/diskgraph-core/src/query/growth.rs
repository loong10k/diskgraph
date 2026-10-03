use crate::DiskNode;

/// 同一路径的前后观测及有符号字节差；来源：DiskGraph 原生 Rust query::Growth。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Growth<'a> {
    pub before: &'a DiskNode,
    pub after: &'a DiskNode,
    pub delta_bytes: i128,
}
