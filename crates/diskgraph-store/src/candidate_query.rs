//! 有界审阅候选查询：筛选工作留在 SQLite，仅读取被选中的节点和证据。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use diskgraph_core::{QueryBudget, TruncationReason};
use rusqlite::{OptionalExtension, params};

use crate::{CandidateSelection, Result, SqliteSnapshotStore, StoreError};

// A blocked node excludes both its ancestors and descendants. The recursive
// sets stay inside SQLite's deadline-bound reader; no O(revision) Rust map is
// constructed. Evidence is read only for selected candidate rows.
const CANDIDATE_SQL: &str = "
WITH RECURSIVE
blocked_seed(id) AS (
    SELECT DISTINCT node_id FROM evidence
    WHERE snapshot_id = ?1
      AND json_extract(evidence_json, '$.relation') IN ('protected', 'used_by_process')
),
blocked_up(id) AS (
    SELECT id FROM blocked_seed
    UNION
    SELECT n.parent_id FROM nodes n JOIN blocked_up b ON n.snapshot_id = ?1 AND n.id = b.id
    WHERE n.parent_id IS NOT NULL
),
blocked_down(id) AS (
    SELECT id FROM blocked_seed
    UNION
    SELECT n.id FROM nodes n JOIN blocked_down b ON n.snapshot_id = ?1 AND n.parent_id = b.id
)
SELECT n.id, n.parent_id, n.subtree_bytes FROM nodes n
WHERE n.snapshot_id = ?1
  AND (n.kind = 'directory' OR (n.kind IS NULL AND json_extract(NULLIF(n.node_json, ''), '$.kind') = 'directory'))
  AND n.subtree_bytes > 0
  AND COALESCE(n.read_error, json_extract(NULLIF(n.node_json, ''), '$.read_error'), 0) = 0
  AND COALESCE(json_extract(NULLIF(n.node_json, ''), '$.size_known'), 1) = 1
  AND EXISTS (
      SELECT 1 FROM evidence e
      WHERE e.snapshot_id = n.snapshot_id AND e.node_id = n.id
        AND json_extract(e.evidence_json, '$.relation') = 'rebuildable'
        AND json_extract(e.evidence_json, '$.confidence') > 0
  )
  AND NOT EXISTS (SELECT 1 FROM blocked_up WHERE id = n.id)
  AND NOT EXISTS (SELECT 1 FROM blocked_down WHERE id = n.id)
ORDER BY n.subtree_bytes DESC, n.id ASC";

fn interrupted(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(fault, _) if fault.code == rusqlite::ErrorCode::OperationInterrupted)
}

impl SqliteSnapshotStore {
    /// 按大小选择不重叠的可审阅目录；仅解码结果节点，期限由只读连接和预算共同约束。
    /// 准备有界审阅候选、目标缺口和真实截断状态。
    /// 参数：snapshot_id：固定快照 ID；target_bytes：审阅目标字节数；budget：节点、字节和期限预算。
    /// 返回：`Result<CandidateSelection>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn candidate_selection(
        &self,
        snapshot_id: &str,
        target_bytes: u64,
        budget: QueryBudget,
    ) -> Result<CandidateSelection> {
        if budget.max_nodes == 0 || budget.max_response_bytes == 0 || budget.deadline_ms == 0 {
            return Err(StoreError::InvalidGraph(
                "candidate budget must be positive".into(),
            ));
        }
        let coverage_complete = self.snapshot(snapshot_id)?.coverage.complete;
        let mut result = CandidateSelection::empty(target_bytes, coverage_complete);
        if target_bytes == 0 || !coverage_complete {
            return Ok(result);
        }

        let deadline = Instant::now() + Duration::from_millis(budget.deadline_ms);
        let mut statement = self.connection.prepare(CANDIDATE_SQL)?;
        let mut rows = match statement.query([snapshot_id]) {
            Ok(rows) => rows,
            Err(error) if interrupted(&error) => {
                result.stop(TruncationReason::Deadline);
                return Ok(result);
            }
            Err(error) => return Err(error.into()),
        };
        let mut parent_statement = self
            .connection
            .prepare("SELECT parent_id FROM nodes WHERE snapshot_id = ?1 AND id = ?2")?;
        let mut selected = HashSet::new();
        let mut selected_lineage = HashSet::new();
        let mut response_bytes = 0_usize;

        loop {
            if Instant::now() >= deadline {
                result.stop(TruncationReason::Deadline);
                break;
            }
            let row = match rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => break,
                Err(error) if interrupted(&error) => {
                    result.stop(TruncationReason::Deadline);
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            let id =
                u64::try_from(row.get::<_, i64>(0)?).map_err(|_| StoreError::IntegerOverflow)?;
            let mut parent = row.get::<_, Option<i64>>(1)?;
            let bytes =
                u64::try_from(row.get::<_, i64>(2)?).map_err(|_| StoreError::IntegerOverflow)?;

            // A selected descendant or ancestor makes this directory overlap.
            if selected_lineage.contains(&id) {
                continue;
            }
            let mut lineage = Vec::new();
            let mut overlaps = false;
            while let Some(parent_id) = parent {
                if Instant::now() >= deadline {
                    result.stop(TruncationReason::Deadline);
                    return Ok(result);
                }
                let parent_id_u64 =
                    u64::try_from(parent_id).map_err(|_| StoreError::IntegerOverflow)?;
                if selected.contains(&parent_id_u64) {
                    overlaps = true;
                    break;
                }
                lineage.push(parent_id_u64);
                parent = match parent_statement
                    .query_row(params![snapshot_id, parent_id], |row| row.get(0))
                    .optional()
                {
                    Ok(Some(parent)) => parent,
                    Ok(None) => {
                        return Err(StoreError::InvalidGraph(
                            "candidate parent is missing".into(),
                        ));
                    }
                    Err(error) if interrupted(&error) => {
                        result.stop(TruncationReason::Deadline);
                        return Ok(result);
                    }
                    Err(error) => return Err(error.into()),
                };
            }
            if overlaps {
                continue;
            }
            if result.candidates.len() >= budget.max_nodes {
                result.stop(TruncationReason::NodeLimit);
                break;
            }
            let node = match self.node(snapshot_id, id) {
                Ok(Some(node)) => node,
                Ok(None) => {
                    return Err(StoreError::InvalidGraph("candidate node is missing".into()));
                }
                Err(error) if error.is_interrupted() => {
                    result.stop(TruncationReason::Deadline);
                    break;
                }
                Err(error) => return Err(error),
            };
            let evidence = match self.evidence(snapshot_id, id) {
                Ok(evidence) => evidence,
                Err(error) if error.is_interrupted() => {
                    result.stop(TruncationReason::Deadline);
                    break;
                }
                Err(error) => return Err(error),
            };
            let charge = serde_json::to_vec(&(&node, &evidence))?
                .len()
                .saturating_add(128);
            if response_bytes.saturating_add(charge) > budget.max_response_bytes {
                result.stop(TruncationReason::ByteLimit);
                break;
            }
            response_bytes += charge;
            result.selected_bytes = result.selected_bytes.saturating_add(bytes);
            result.remaining_bytes = target_bytes.saturating_sub(result.selected_bytes);
            result.candidates.push((node, evidence));
            selected.insert(id);
            selected_lineage.insert(id);
            selected_lineage.extend(lineage);
            if result.remaining_bytes == 0 {
                break;
            }
        }
        Ok(result)
    }
}
