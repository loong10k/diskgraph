//! 树读取复用请求期限、实际解码账本和最终授权，不持共享图写锁。

use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, DiskNode, PrincipalId, QueryBudget, QueryReadBudget, ScopeId,
    TreeView, TruncationReason, measure_json_bounded, query_deadline,
};
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use std::collections::HashMap;
use std::time::Instant;

impl Engine {
    /// 按真实 scope 授权后读取有预算的树。
    /// 参数：scope/revision、请求身份、depth/min_bytes 为约束。
    /// 返回：兼容树 JSON 或授权/预算失败，首次准备计入默认期限。
    pub fn tree_view(
        &self,
        scope_id: &ScopeId,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        depth: usize,
        min_bytes: u64,
    ) -> Result<TreeView, EngineError> {
        let budget = QueryBudget::default();
        self.tree_view_until(
            scope_id,
            revision_id,
            principal,
            authorizer,
            depth,
            min_bytes,
            budget,
            query_deadline(budget)?,
        )
    }

    /// 复用一个已授权 reader，在编码之后重查真实 scope/grant 与共同期限。
    /// 参数：scope/revision/identity 为请求，depth/min_bytes/budget 为范围，deadline 为最外层期限。
    /// 返回：预算内兼容树与截断原因；授权失败不得返回部分树。
    #[allow(clippy::too_many_arguments)] // 旧上下文与独立范围/预算均为必需参数。
    pub fn tree_view_until(
        &self,
        scope: &ScopeId,
        revision: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        depth: usize,
        min_bytes: u64,
        budget: QueryBudget,
        deadline: Instant,
    ) -> Result<TreeView, EngineError> {
        self.tree_view_with_finish_until(
            scope,
            revision,
            principal,
            authorizer,
            depth,
            min_bytes,
            budget,
            deadline,
            |_, _| Ok(()),
        )
    }

    /// 在原树请求的连续撤权见证内完成适配器的实际编码。
    /// 参数：范围、身份、预算和期限沿用 tree_view_until；finish 接收树与原数据期限是否耗尽。
    /// 返回：编码后仍获授权的树；finish 可能因期限转为 partial 被再次调用，不得发布文件或响应。
    #[allow(clippy::too_many_arguments)] // 保留原请求参数，仅增加受同一授权生命周期保护的编码回调。
    pub fn tree_view_with_finish_until(
        &self,
        scope: &ScopeId,
        revision: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        depth: usize,
        min_bytes: u64,
        budget: QueryBudget,
        deadline: Instant,
        mut finish: impl FnMut(&mut TreeView, bool) -> Result<(), EngineError>,
    ) -> Result<TreeView, EngineError> {
        self.with_relation_reader_until(
            revision,
            principal,
            authorizer,
            deadline,
            budget,
            Some(scope),
            |reader, snapshot, reads| {
                let snapshot = snapshot.ok_or(BusinessError::BudgetExceeded)?;
                tree_on_reader(
                    reader,
                    snapshot.snapshot_id(),
                    depth,
                    min_bytes,
                    budget,
                    reads,
                )
            },
            |tree, expired| {
                if expired {
                    tree.root["truncated"] = serde_json::json!(true);
                    tree.root["truncation_reason"] = serde_json::json!("deadline");
                }
                if measure_json_bounded(&tree.root, budget.max_response_bytes)
                    .map_err(StoreError::from)?
                    .is_none()
                {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                finish(tree, expired)
            },
        )
    }

    /// 可信内部按默认起点读取有界树。
    /// 参数：revision/depth/min_bytes/budget 须由调用方先授权。
    /// 返回：兼容树与明确截断诊断；不会重新加载完整 revision。
    pub fn tree_view_bounded(
        &self,
        revision: &str,
        depth: usize,
        min_bytes: u64,
        budget: QueryBudget,
    ) -> Result<TreeView, EngineError> {
        self.tree_view_bounded_until(revision, depth, min_bytes, budget, query_deadline(budget)?)
    }

    /// 可信树读取沿用首次准备前的共同期限。
    /// 参数：revision/depth/min_bytes/budget/deadline 为固定查询；调用方负责首末授权。
    /// 返回：真实编码额度内的树，节点数量统计实际解码而不是仅展示条数。
    pub fn tree_view_bounded_until(
        &self,
        revision: &str,
        depth: usize,
        min_bytes: u64,
        budget: QueryBudget,
        deadline: Instant,
    ) -> Result<TreeView, EngineError> {
        let mut reads = QueryReadBudget::new(budget, deadline)?;
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let snapshot = reader.revision_snapshot_with_budget(revision, &mut reads)?;
        tree_on_reader(&reader, &snapshot, depth, min_bytes, budget, &mut reads)
    }
}

fn tree_on_reader(
    reader: &SqliteSnapshotStore,
    snapshot: &str,
    depth: usize,
    minimum: u64,
    budget: QueryBudget,
    ledger: &mut QueryReadBudget,
) -> Result<TreeView, EngineError> {
    let root = reader
        .root_node_with_budget(snapshot, ledger)?
        .ok_or(BusinessError::NotFound)?;
    let mut nodes = vec![(root, 0usize)];
    let mut children = HashMap::new();
    let mut counts = HashMap::new();
    let mut index = 0;
    while index < nodes.len() {
        if !ledger.check() {
            break;
        }
        let (id, level) = (nodes[index].0.id, nodes[index].1);
        let (all, kept) = match reader.tree_child_counts(snapshot, id, minimum) {
            Ok(value) => value,
            Err(error) if error.is_interrupted() => {
                ledger.stop(TruncationReason::Deadline);
                break;
            }
            Err(error) => return Err(error.into()),
        };
        counts.insert(id, (all, kept));
        if level < depth.min(budget.max_depth) && kept > 0 {
            let page = reader.tree_children_with_budget(
                snapshot,
                id,
                minimum,
                ledger.remaining_nodes(),
                ledger,
            );
            let (page, more) = match page {
                Ok(page) => page,
                Err(error) if error.is_interrupted() => {
                    ledger.stop(TruncationReason::Deadline);
                    break;
                }
                Err(StoreError::BudgetExceeded) => break,
                Err(error) => return Err(error.into()),
            };
            let mut ids = Vec::new();
            for node in page {
                ids.push(node.id);
                nodes.push((node, level + 1));
            }
            children.insert(id, ids);
            if more && ledger.stopped().is_none() {
                ledger.stop(TruncationReason::NodeLimit);
            }
        }
        index += 1;
    }
    ledger.check();
    let mut reason = ledger.stopped().map(tree_reason);
    loop {
        let root = render_nodes(
            &nodes,
            &children,
            &counts,
            depth.min(budget.max_depth),
            reason,
            ledger.nodes_read(),
        );
        if measure_json_bounded(&root, budget.max_response_bytes)
            .map_err(StoreError::from)?
            .is_some()
        {
            return Ok(TreeView { root });
        }
        if nodes.len() == 1 {
            return Err(BusinessError::BudgetExceeded.into());
        }
        nodes.pop();
        reason = Some("response_byte_limit");
    }
}

fn tree_reason(reason: TruncationReason) -> &'static str {
    match reason {
        TruncationReason::Deadline => "deadline",
        TruncationReason::ByteLimit => "response_byte_limit",
        TruncationReason::NodeLimit => "node_limit",
        TruncationReason::DepthLimit => "depth_limit",
        TruncationReason::EdgeLimit => "edge_limit",
    }
}

fn render_nodes(
    nodes: &[(DiskNode, usize)],
    children: &HashMap<u64, Vec<u64>>,
    counts: &HashMap<u64, (u64, u64)>,
    depth: usize,
    reason: Option<&'static str>,
    nodes_read: usize,
) -> serde_json::Value {
    let mut rendered = HashMap::new();
    for (node, level) in nodes.iter().rev() {
        let mut value = serde_json::json!({"name":node.name,"kind":node.kind,"size_bytes":node.subtree_bytes,"own_bytes":node.direct_bytes,"files":node.files,"dirs":node.directories});
        if !node.size_known {
            value["size_known"] = serde_json::json!(false);
        }
        if let Some(category) = &node.category_hint {
            value["category_hint"] = serde_json::json!(category);
        }
        if node.read_error {
            value["read_error"] = serde_json::json!(true);
        }
        let (all, kept) = counts.get(&node.id).copied().unwrap_or((0, 0));
        if *level < depth && all > 0 {
            let entries = children
                .get(&node.id)
                .into_iter()
                .flatten()
                .filter_map(|id| rendered.remove(id))
                .collect::<Vec<_>>();
            let shown = entries.len() as u64;
            value["children"] = serde_json::json!(entries);
            if all > kept {
                value["hidden_below_min_bytes"] = serde_json::json!(all - kept);
            }
            if shown < kept {
                value["truncated"] = serde_json::json!(true);
                value["children_count"] = serde_json::json!(all);
            }
        } else if all > 0 {
            value["truncated"] = serde_json::json!(true);
            value["children_count"] = serde_json::json!(all);
        }
        rendered.insert(node.id, value);
    }
    let mut root = rendered
        .remove(&nodes[0].0.id)
        .expect("root remains in the bounded tree");
    if let Some(reason) = reason {
        root["truncated"] = serde_json::json!(true);
        root["truncation_reason"] = serde_json::json!(reason);
        root["nodes_read"] = serde_json::json!(nodes_read);
    }
    root
}
