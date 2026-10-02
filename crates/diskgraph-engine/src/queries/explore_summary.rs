//! 借用目录节点/覆盖信息并返回有预算的直接子节点和截断原因。

use diskgraph_core::{BudgetTracker, DiskGraph, DiskNode, QueryBudget, TruncationReason};

/// 借用目录节点/覆盖信息并返回有预算的直接子节点和截断原因。
/// 来源：原生 Rust diskgraph-engine::ExploreSummary。
/// A bounded directory/relation summary: the node plus its direct children
/// ranked by observed size, with explicit coverage and truncation.
pub struct ExploreSummary<'a> {
    pub node: &'a DiskNode,
    pub children: Vec<&'a DiskNode>,
    pub coverage: &'a diskgraph_core::ScanCoverage,
    pub truncated: Option<TruncationReason>,
}
/// 借用一层目录与覆盖信息，明确预算截断。
/// 参数：graph/node_id 为图和节点，budget 为读取预算。
/// 返回：直接子节点摘要；保留未知节点及无效预算的旧回退。
pub fn explore<'a>(graph: &'a DiskGraph, node_id: u64, budget: QueryBudget) -> ExploreSummary<'a> {
    let mut tracker = BudgetTracker::new(budget).unwrap_or_else(|_| {
        BudgetTracker::new(QueryBudget::default()).expect("default budget is valid")
    });
    let node = graph
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .unwrap_or(&graph.nodes[0]);
    let max_children = tracker.budget().max_nodes;
    let ranked = graph.top(node.id, max_children);
    let hit_cap = ranked.len() >= max_children;
    let mut children: Vec<&DiskNode> = Vec::new();
    for child in &ranked {
        if !tracker.charge_node() {
            break;
        }
        children.push(*child);
    }
    ExploreSummary {
        node,
        coverage: &graph.snapshot.coverage,
        truncated: tracker.truncated().or(if hit_cap {
            Some(TruncationReason::NodeLimit)
        } else {
            None
        }),
        children,
    }
}
