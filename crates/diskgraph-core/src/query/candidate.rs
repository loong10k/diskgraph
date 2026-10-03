use crate::{DiskNode, EvidenceEdge};

/// 只供审阅的节点及重建证据，不授予删除权限；来源：DiskGraph 原生 Rust query::Candidate。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate<'a> {
    pub node: &'a DiskNode,
    pub evidence: Vec<&'a EvidenceEdge>,
}
