use crate::{Result, StoreError};
use diskgraph_core::DiskGraph;
use std::collections::{HashMap, HashSet};

/// 验证图身份、父子关系、定位与覆盖一致性。
/// 参数：graph：完整观测图。
/// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
pub(crate) fn validate_graph(graph: &DiskGraph) -> Result<()> {
    validate_graph_display_aliases(graph, false).map(|_| ())
}

/// 有无损暂存身份的发布可允许显示别名；原始身份唯一性由同一发布事务检查。
/// 参数：graph 为完整观测图，allow_aliases 表示发布事务另行验证原始定位唯一性。
/// 返回：图结构有效时返回显示别名标记；原始定位仍须在发布事务中另行验证。
/// 身份、父节点或覆盖冲突返回 InvalidGraph，不改变各拒绝条件的顺序。
pub(crate) fn validate_graph_display_aliases(
    graph: &DiskGraph,
    allow_aliases: bool,
) -> Result<bool> {
    if graph.snapshot.id.is_empty() || graph.nodes.is_empty() {
        return Err(StoreError::InvalidGraph(
            "missing snapshot ID or nodes".into(),
        ));
    }
    // 显示定位只计一次；临时集合在 ID 表和颜色表分配前释放，发布调用方复用纯标记。
    // 标记不是可信身份或授权，仍按原顺序完成结构检查后才判断是否允许别名。
    let has_display_aliases = graph
        .nodes
        .iter()
        .map(|node| &node.locator)
        .collect::<HashSet<_>>()
        .len()
        != graph.nodes.len();
    let ids: HashMap<_, _> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();
    if ids.len() != graph.nodes.len() {
        return Err(StoreError::InvalidGraph("duplicate node IDs".into()));
    }
    if graph.nodes.iter().any(|node| {
        node.parent_id
            .is_some_and(|parent| !ids.contains_key(&parent))
    }) {
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
    // 每个节点至多进入一次灰色路径、一次完成路径；不递归、不逐节点重走整个祖先链。
    // 父 ID 已全部核验存在；有限无环父链必然终止于上面确认的唯一根。
    let mut colors = vec![0_u8; graph.nodes.len()];
    for start in 0..graph.nodes.len() {
        let mut current = Some(start);
        while let Some(index) = current {
            match colors[index] {
                2 => break,
                1 => return Err(StoreError::InvalidGraph("parent cycle".into())),
                _ => {
                    colors[index] = 1;
                    current = graph.nodes[index].parent_id.map(|parent| ids[&parent]);
                }
            }
        }
        let mut current = Some(start);
        while let Some(index) = current {
            if colors[index] == 2 {
                break;
            }
            colors[index] = 2;
            current = graph.nodes[index].parent_id.map(|parent| ids[&parent]);
        }
    }
    if !allow_aliases && has_display_aliases {
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
        .any(|edge| !ids.contains_key(&edge.node_id) || edge.confidence > 100)
    {
        return Err(StoreError::InvalidGraph("invalid evidence".into()));
    }
    Ok(has_display_aliases)
}
