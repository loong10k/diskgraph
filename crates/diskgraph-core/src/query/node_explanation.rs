use crate::{DiskNode, EvidenceEdge};

/// 节点与解释它的已记录证据；来源：DiskGraph 原生 Rust query::NodeExplanation。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeExplanation<'a> {
    pub node: &'a DiskNode,
    pub evidence: Vec<&'a EvidenceEdge>,
}
