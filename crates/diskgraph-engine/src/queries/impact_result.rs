//! 保存已遍历影响条目及未完成原因。

use super::ImpactEntry;
use diskgraph_core::TruncationReason;

/// 保存已遍历影响条目及未完成原因。
/// 来源：原生 Rust diskgraph-engine::ImpactResult。
/// Impact entries together with the reason traversal stopped before completion.
pub struct ImpactResult {
    pub entries: Vec<ImpactEntry>,
    pub truncated: Option<TruncationReason>,
}
