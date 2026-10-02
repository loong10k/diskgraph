//! 共享 Engine 的 tree_queries 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, ScopeId};
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use std::collections::HashMap;

impl Engine {
    /// 按真实 scope 授权后读取有预算的树。
    /// 参数：scope/revision、请求身份、depth/min_bytes 为约束。
    /// 返回：树视图或授权/预算失败。
    /// A depth-bounded tree view of a published revision. Uses the store's
    /// narrow read path: no JSON payloads, no locators, no full DiskNode
    /// materialization. Pre-v4 snapshots (NULL structured columns) fall
    /// back to the full load so older history renders identically.
    pub fn tree_view(
        &self,
        scope_id: &ScopeId,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        depth: usize,
        min_bytes: u64,
    ) -> Result<diskgraph_core::TreeView, EngineError> {
        self.authorize_revision(Some(scope_id), revision_id, principal, authorizer)?;
        self.tree_view_bounded(
            revision_id,
            depth,
            min_bytes,
            diskgraph_core::QueryBudget::default(),
        )
    }
}

impl Engine {
    /// 可信按层读取并执行节点、时间和响应预算。
    /// 参数：revision、depth、min_bytes、budget 须由调用方先授权。
    /// 返回：兼容树 JSON 及明确截断诊断。
    /// 按层读取树；节点、期限和响应字节预算耗尽时返回明确截断诊断。
    /// 可信内部调用必须先经 authorize_revision 验证请求主体。
    pub fn tree_view_bounded(
        &self,
        revision_id: &str,
        depth: usize,
        min_bytes: u64,
        budget: diskgraph_core::QueryBudget,
    ) -> Result<diskgraph_core::TreeView, EngineError> {
        let budget = budget.validated()?;
        let reader = SqliteSnapshotStore::open_reader(&self.graph_path, budget.deadline_ms, None)?;
        let snapshot = reader.revision(revision_id)?.snapshot_id;
        let root = reader
            .root_node(&snapshot)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let root_id = root.id;
        let started = std::time::Instant::now();
        let mut nodes = vec![(root, 0usize)];
        let mut children = HashMap::<u64, Vec<u64>>::new();
        let mut counts = HashMap::<u64, (u64, u64)>::new();
        let mut bytes = serde_json::to_vec(&nodes[0].0)
            .map_err(StoreError::from)?
            .len();
        if bytes > budget.max_response_bytes {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let mut reason = None;
        let mut index = 0;
        while index < nodes.len() {
            let (node, level) = &nodes[index];
            let (id, level) = (node.id, *level);
            if started.elapsed().as_millis() >= u128::from(budget.deadline_ms) {
                reason = Some("deadline");
                break;
            }
            let (all, kept) = match reader.child_counts(&snapshot, id, min_bytes) {
                Ok(counts) => counts,
                Err(error) if error.is_interrupted() => {
                    reason = Some("deadline");
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            counts.insert(id, (all, kept));
            if level >= depth.min(budget.max_depth) {
                index += 1;
                continue;
            }
            if nodes.len() >= budget.max_nodes {
                if kept > 0 {
                    reason = Some("node_limit");
                }
                index += 1;
                continue;
            }
            let remaining = budget.max_nodes - nodes.len();
            let page =
                match reader.children_page(&snapshot, id, Some(min_bytes), 0, remaining as u64) {
                    Ok(page) => page,
                    Err(error) if error.is_interrupted() => {
                        reason = Some("deadline");
                        break;
                    }
                    Err(error) => return Err(error.into()),
                };
            // tree 保留未知大小对象；children_page 的已知大小过滤仅用于普通目录列表。
            let page = if page.2 > 0 {
                match reader.children(&snapshot, id, 0, remaining as u64) {
                    Ok(page) => page,
                    Err(error) if error.is_interrupted() => {
                        reason = Some("deadline");
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
            } else {
                page.0
            };
            let mut ids = Vec::new();
            for child in page {
                if child.subtree_bytes < min_bytes {
                    continue;
                }
                let size = serde_json::to_vec(&child).map_err(StoreError::from)?.len();
                if bytes.saturating_add(size) > budget.max_response_bytes {
                    reason = Some("response_byte_limit");
                    break;
                }
                bytes += size;
                ids.push(child.id);
                nodes.push((child, level + 1));
            }
            if (ids.len() as u64) < kept && reason.is_none() {
                reason = Some("node_limit");
            }
            children.insert(id, ids);
            index += 1;
            if reason == Some("response_byte_limit") {
                break;
            }
        }
        let nodes_read = nodes.len();
        let mut rendered = HashMap::<u64, serde_json::Value>::new();
        for (node, level) in nodes.into_iter().rev() {
            let mut value = serde_json::json!({"name":node.name, "kind":node.kind, "size_bytes":node.subtree_bytes, "own_bytes":node.direct_bytes, "files":node.files, "dirs":node.directories});
            if let Some(category) = node.category_hint {
                value["category_hint"] = serde_json::json!(category);
            }
            if node.read_error {
                value["read_error"] = serde_json::json!(true);
            }
            let (all, kept) = counts.get(&node.id).copied().unwrap_or((0, 0));
            let ids = children.remove(&node.id).unwrap_or_default();
            if level < depth.min(budget.max_depth) && all > 0 {
                let shown = ids.len() as u64;
                value["children"] = serde_json::json!(
                    ids.into_iter()
                        .filter_map(|id| rendered.remove(&id))
                        .collect::<Vec<_>>()
                );
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
            .remove(&root_id)
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        if let Some(reason) = reason {
            root["truncated"] = serde_json::json!(true);
            root["truncation_reason"] = serde_json::json!(reason);
            root["nodes_read"] = serde_json::json!(nodes_read);
        }
        Ok(diskgraph_core::TreeView { root })
    }
}
