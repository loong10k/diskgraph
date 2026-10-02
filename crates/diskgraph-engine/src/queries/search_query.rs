//! 保留旧内存图搜索的兼容语义。

use diskgraph_core::{BudgetTracker, DiskGraph, DiskNode, QueryBudget};

/// 按现有 Unicode 小写子串语义稳定搜索。
/// 参数：graph/pattern、offset/limit 与 budget 为内存图查询。
/// 返回：借用节点页及 next offset；不推断用户意图。
/// Name/path pattern search over one revision. Matching is a pattern lookup,
/// never an intent guess; ambiguity is returned as several candidates.
pub fn search_nodes<'a>(
    graph: &'a DiskGraph,
    pattern: &str,
    offset: u64,
    limit: usize,
    budget: QueryBudget,
) -> (Vec<&'a DiskNode>, Option<u64>) {
    let mut tracker = match BudgetTracker::new(budget) {
        Ok(tracker) => tracker,
        Err(_) => return (Vec::new(), None),
    };
    let needle = pattern.to_lowercase();
    let mut matches: Vec<&DiskNode> = graph
        .nodes
        .iter()
        .filter(|node| {
            node.name.to_lowercase().contains(&needle)
                || match &node.locator {
                    diskgraph_core::ResourceLocator::NativePath(path) => {
                        path.to_lowercase().contains(&needle)
                    }
                    diskgraph_core::ResourceLocator::DocumentUri(uri) => {
                        uri.to_lowercase().contains(&needle)
                    }
                }
        })
        .collect();
    matches.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let mut out: Vec<&DiskNode> = Vec::new();
    let total = matches.len() as u64;
    let mut next_offset = offset;
    for (index, node) in matches.into_iter().enumerate() {
        if (index as u64) < offset {
            continue;
        }
        if out.len() >= limit || !tracker.charge_node() {
            next_offset = offset + out.len() as u64;
            break;
        }
        out.push(node);
    }
    // A page that stops short of the total still offers a continuation.
    let consumed = offset + out.len() as u64;
    let next = if consumed < total {
        Some(consumed)
    } else {
        None
    };
    let _ = next_offset;
    (out, next)
}
