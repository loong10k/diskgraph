//! 共享 Engine 的 revision_comparison 职责；原调用与持锁顺序保持。

use crate::native_locator::native_path;
use crate::{CompareRow, ComparisonReport, Engine, EngineError};
use diskgraph_core::BusinessError;
use diskgraph_store::SqliteSnapshotStore;

impl Engine {
    /// 可信内部以默认预算有序合并两份历史。
    /// 参数：left/right revision 与 tolerance_seconds 指定比较。
    /// 返回：比较报告；调用方须先授权双侧。
    /// 以有序路径游标比较两份历史，不同时加载两棵完整树；截断时统计明确为局部。
    pub fn compare_revisions(
        &self,
        left_revision: &str,
        right_revision: &str,
        tolerance_seconds: i64,
    ) -> Result<ComparisonReport, EngineError> {
        self.compare_revisions_bounded(
            left_revision,
            right_revision,
            tolerance_seconds,
            diskgraph_core::QueryBudget::default(),
        )
    }
}

impl Engine {
    /// 可信内部在节点、时间和响应预算内合并历史。
    /// 参数：双侧 revision、时间容差与 budget 为比较约束。
    /// 返回：比较报告；截断时统计为局部，以 truncated/node_counts_complete 区分完成状态。
    /// 使用指定节点、时间和响应预算执行历史比较，返回已逐条产出的结果。
    pub fn compare_revisions_bounded(
        &self,
        left_revision: &str,
        right_revision: &str,
        tolerance_seconds: i64,
        budget: diskgraph_core::QueryBudget,
    ) -> Result<ComparisonReport, EngineError> {
        let budget = budget.validated()?;
        let left = SqliteSnapshotStore::open_reader(&self.graph_path, budget.deadline_ms, None)?;
        let right = SqliteSnapshotStore::open_reader(&self.graph_path, budget.deadline_ms, None)?;
        let left_snapshot = left.revision(left_revision)?.snapshot_id;
        let right_snapshot = right.revision(right_revision)?.snapshot_id;
        let left_root = left
            .root_node(&left_snapshot)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let right_root = right
            .root_node(&right_snapshot)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let left_path = native_path(&left_root)?;
        let right_path = native_path(&right_root)?;
        let started = std::time::Instant::now();
        let mut rows = Vec::new();
        let mut summary = diskgraph_core::Summary::default();
        let result = left.with_ordered_nodes(&left_snapshot, &left_path, |left_iter| {
            right.with_ordered_nodes(&right_snapshot, &right_path, |right_iter| {
                let mut current_left = left_iter.next().transpose()?;
                let mut current_right = right_iter.next().transpose()?;

                let mut bytes = 0usize;
                let mut reason = None;
                while current_left.is_some() || current_right.is_some() {
                    if rows.len() >= budget.max_nodes {
                        reason = Some("node_limit");
                        break;
                    }
                    if started.elapsed().as_millis() >= u128::from(budget.deadline_ms) {
                        reason = Some("deadline");
                        break;
                    }
                    let ordering = match (&current_left, &current_right) {
                        (Some(left), Some(right)) => left.0.cmp(&right.0),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        _ => break,
                    };
                    let on_left = if ordering != std::cmp::Ordering::Greater {
                        current_left.take()
                    } else {
                        None
                    };
                    let on_right = if ordering != std::cmp::Ordering::Less {
                        current_right.take()
                    } else {
                        None
                    };
                    let path = on_left
                        .as_ref()
                        .or(on_right.as_ref())
                        .expect("at least one node exists")
                        .0
                        .clone();
                    let verdict = match (&on_left, &on_right) {
                        (Some(left), Some(right)) => diskgraph_core::compare::compare_entry(
                            &left.1,
                            &right.1,
                            tolerance_seconds,
                        ),
                        (Some(_), None) => diskgraph_core::Verdict::LeftOnly,
                        _ => diskgraph_core::Verdict::RightOnly,
                    };
                    let row = CompareRow {
                        path,
                        verdict,
                        left_bytes: on_left.as_ref().map(|node| node.1.subtree_bytes),
                        right_bytes: on_right.as_ref().map(|node| node.1.subtree_bytes),
                        is_file: on_left
                            .as_ref()
                            .or(on_right.as_ref())
                            .is_some_and(|node| node.1.kind == diskgraph_core::NodeKind::File),
                        digests: None,
                    };
                    let size = serde_json::to_vec(&row)?.len();
                    if bytes.saturating_add(size) > budget.max_response_bytes {
                        reason = Some("response_byte_limit");
                        break;
                    }
                    bytes += size;
                    match &row.verdict {
                        diskgraph_core::Verdict::LeftOnly => summary.left_only += 1,
                        diskgraph_core::Verdict::RightOnly => summary.right_only += 1,
                        diskgraph_core::Verdict::Same { .. } => summary.same += 1,
                        diskgraph_core::Verdict::Different { reason } => {
                            summary.different += 1;
                            if *reason == diskgraph_core::DifferentReason::UnknownSize {
                                summary.unknown += 1;
                            }
                        }
                    }
                    rows.push(row);
                    if ordering != std::cmp::Ordering::Greater {
                        current_left = left_iter.next().transpose()?;
                    }
                    if ordering != std::cmp::Ordering::Less {
                        current_right = right_iter.next().transpose()?;
                    }
                }
                Ok(reason)
            })
        });
        let mut truncated = match result {
            Ok(reason) => reason,
            Err(error) if error.is_interrupted() => Some("deadline"),
            Err(error) => return Err(error.into()),
        };
        let mut node_counts_complete = true;
        let mut count =
            |store: &SqliteSnapshotStore, snapshot: &str| -> Result<usize, EngineError> {
                match store.node_count(snapshot) {
                    Ok(value) => Ok(value as usize),
                    Err(error) if error.is_interrupted() => {
                        node_counts_complete = false;
                        truncated = Some("deadline");
                        Ok(0)
                    }
                    Err(error) => Err(error.into()),
                }
            };
        let left_nodes = count(&left, &left_snapshot)?;
        let right_nodes = count(&right, &right_snapshot)?;
        Ok(ComparisonReport {
            left_revision: left_revision.to_owned(),
            right_revision: right_revision.to_owned(),
            left_root: left_root.locator,
            right_root: right_root.locator,
            left_nodes,
            right_nodes,
            node_counts_complete,
            rows,
            summary,
            truncated,
        })
    }
}
