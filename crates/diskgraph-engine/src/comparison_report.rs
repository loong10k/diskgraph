//! 保存双方 revision/根、节点计数、逐路径差异及局部统计，截断不代表完整比较。

use crate::CompareRow;

/// 保存双方 revision/根、节点计数、逐路径差异及局部统计，截断不代表完整比较。
/// 来源：原生 Rust diskgraph-engine::ComparisonReport。
/// The result of comparing two revisions: what was compared, and what differs.
///
/// Carries both roots and both node counts so a caller can tell a comparison
/// of two checkouts from a comparison of two million-file trees, and never
/// has to assume the second.
pub struct ComparisonReport {
    pub left_revision: String,
    pub right_revision: String,
    pub left_root: diskgraph_core::ResourceLocator,
    pub right_root: diskgraph_core::ResourceLocator,
    pub left_nodes: usize,
    pub right_nodes: usize,
    /// 节点总数是否完成；查询期限中断时 0 不代表空 revision。
    pub node_counts_complete: bool,
    pub rows: Vec<CompareRow>,
    pub summary: diskgraph_core::Summary,
    /// 截断时 summary 仅描述已比较条目，不能解释为完整统计。
    pub truncated: Option<&'static str>,
}

impl ComparisonReport {
    /// 输出比较报告的既有字段并标注局部统计。
    /// 参数：limit 限制显示的逐路径条目。
    /// 返回：比较 JSON；显示上限不改变已完成比较状态。
    /// The shape a caller prints, in the order the fields matter.
    pub fn to_json(&self, limit: Option<usize>) -> serde_json::Value {
        let shown: Vec<serde_json::Value> = self
            .rows
            .iter()
            .take(limit.unwrap_or(self.rows.len()))
            .map(CompareRow::to_json)
            .collect();
        serde_json::json!({
            "left": {
                "revision_id": self.left_revision,
                "root": self.left_root,
                "nodes": self.left_nodes,
            },
            "right": {
                "revision_id": self.right_revision,
                "root": self.right_root,
                "nodes": self.right_nodes,
            },
            "node_counts_complete": self.node_counts_complete,
            "summary": self.summary,
            "complete": self.truncated.is_none(),
            "summary_is_partial": self.truncated.is_some(),
            "truncation_reason": self.truncated,
            "actionable": self.summary.actionable(),
            "entries": shown.len(),
            "rows": shown,
        })
    }
}
