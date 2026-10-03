use super::{TreeNode, TreeRenderError};
use crate::DiskGraph;

/// 深度有界的树视图，标记截断及隐藏子项，不声称展示全部节点；来源：DiskGraph 原生 Rust query::TreeView。
/// A depth-bounded tree view of one published revision, in the shape a tree
/// UI consumes: each node carries its aggregate size, its own bytes, and
/// children sorted largest-first. Cutting at the depth bound is reported
/// with `truncated: true` and the child count, so the JSON never claims to
/// have shown more than it did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeView {
    pub root: serde_json::Value,
}

/// 从图节点构造深度有界的展示树，不改变原图。
/// 参数：graph 为观测图，depth 为展示深度，min_bytes 为子树过滤阈值。
/// 返回：带深度截断及隐藏项计数的展示树，缺根时返回错误。
pub fn render_tree(
    graph: &DiskGraph,
    depth: usize,
    min_bytes: u64,
) -> Result<TreeView, TreeRenderError> {
    let rows: Vec<TreeNode<'_>> = graph
        .nodes
        .iter()
        .map(|node| TreeNode {
            id: node.id,
            parent_id: node.parent_id,
            name: node.name.as_str(),
            kind: node.kind,
            subtree_bytes: node.subtree_bytes,
            direct_bytes: node.direct_bytes,
            files: node.files,
            directories: node.directories,
            read_error: node.read_error,
            category_hint: node.category_hint.as_deref(),
        })
        .collect();
    render_tree_rows(&rows, depth, min_bytes)
}

/// Renders a tree view from narrow node rows (the store's fast read path).
/// 从窄行构造展示树，子节点按大小降序排列，截断时明确报告。
/// 参数：rows 为节点窄行，depth 为最大层数，min_bytes 为子树过滤阈值。
/// 返回：展示树或缺失根节点错误。
pub fn render_tree_rows(
    rows: &[TreeNode<'_>],
    depth: usize,
    min_bytes: u64,
) -> Result<TreeView, TreeRenderError> {
    use serde_json::json;

    let root = rows
        .iter()
        .find(|node| node.parent_id.is_none())
        .ok_or(TreeRenderError::NoRoot)?;
    let children_of: std::collections::HashMap<u64, Vec<&TreeNode<'_>>> =
        rows.iter()
            .fold(std::collections::HashMap::new(), |mut map, node| {
                if let Some(parent) = node.parent_id {
                    map.entry(parent).or_default().push(node);
                }
                map
            });

    fn render(
        current: &TreeNode<'_>,
        children_of: &std::collections::HashMap<u64, Vec<&TreeNode<'_>>>,
        depth: usize,
        max_depth: usize,
        min_bytes: u64,
    ) -> serde_json::Value {
        let mut value = json!({
            "name": current.name,
            "kind": current.kind,
            "size_bytes": current.subtree_bytes,
            "own_bytes": current.direct_bytes,
            "files": current.files,
            "dirs": current.directories,
        });
        // The category is what a view colours by; a tree that drops it cannot
        // show a legend.
        if let Some(category) = current.category_hint {
            value["category_hint"] = json!(category);
        }
        if current.read_error {
            value["read_error"] = json!(true);
        }
        let kids = children_of.get(&current.id).cloned().unwrap_or_default();
        if depth >= max_depth || kids.is_empty() {
            if !kids.is_empty() {
                value["truncated"] = json!(true);
                value["children_count"] = json!(kids.len());
            }
            return value;
        }
        let kept: Vec<_> = kids
            .iter()
            .filter(|kid| kid.subtree_bytes >= min_bytes)
            .cloned()
            .collect();
        let hidden = kids.len() - kept.len();
        let mut ordered = kept;
        ordered.sort_by_key(|kid| (std::cmp::Reverse(kid.subtree_bytes), kid.name));
        value["children"] = json!(
            ordered
                .iter()
                .map(|kid| render(kid, children_of, depth + 1, max_depth, min_bytes))
                .collect::<Vec<_>>()
        );
        if hidden > 0 {
            value["hidden_below_min_bytes"] = json!(hidden);
        }
        value
    }

    Ok(TreeView {
        root: render(root, &children_of, 1, depth, min_bytes),
    })
}
