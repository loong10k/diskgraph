use crate::{Result, StoreError};
use diskgraph_core::DiskGraph;
use std::collections::HashSet;

/// 验证图身份、父子关系、定位与覆盖一致性。
/// 参数：graph：完整观测图。
/// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
pub(crate) fn validate_graph(graph: &DiskGraph) -> Result<()> {
    validate_graph_display_aliases(graph, false)
}

/// 有无损暂存身份的发布可允许显示别名；原始身份唯一性由同一发布事务检查。
/// 参数：graph 为完整观测图，allow_aliases 表示发布事务另行验证原始定位唯一性。
/// 返回：图结构有效时成功；身份、父节点或覆盖冲突返回 InvalidGraph。
pub(crate) fn validate_graph_display_aliases(graph: &DiskGraph, allow_aliases: bool) -> Result<()> {
    if graph.snapshot.id.is_empty() || graph.nodes.is_empty() {
        return Err(StoreError::InvalidGraph(
            "missing snapshot ID or nodes".into(),
        ));
    }
    let ids: HashSet<_> = graph.nodes.iter().map(|node| node.id).collect();
    if ids.len() != graph.nodes.len() {
        return Err(StoreError::InvalidGraph("duplicate node IDs".into()));
    }
    if graph
        .nodes
        .iter()
        .any(|node| node.parent_id.is_some_and(|parent| !ids.contains(&parent)))
    {
        return Err(StoreError::InvalidGraph("missing parent node".into()));
    }
    let roots: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.parent_id.is_none())
        .collect();
    if roots.len() != 1 || roots[0].locator != graph.snapshot.root {
        return Err(StoreError::InvalidGraph(
            "snapshot must have one matching root".into(),
        ));
    }
    let locators: HashSet<_> = graph.nodes.iter().map(|node| &node.locator).collect();
    if !allow_aliases && locators.len() != graph.nodes.len() {
        return Err(StoreError::InvalidGraph(
            "duplicate resource locators".into(),
        ));
    }
    if graph.snapshot.coverage.complete
        && (graph.snapshot.coverage.unreadable_nodes > 0 || graph.snapshot.coverage.depth_limited)
    {
        return Err(StoreError::InvalidGraph(
            "inconsistent scan coverage".into(),
        ));
    }
    if graph
        .evidence
        .iter()
        .any(|edge| !ids.contains(&edge.node_id) || edge.confidence > 100)
    {
        return Err(StoreError::InvalidGraph("invalid evidence".into()));
    }
    Ok(())
}
