//! 有界审阅候选查询：筛选工作留在 SQLite，仅读取被选中的节点和证据。

use std::collections::HashSet;
use std::time::Instant;

use diskgraph_core::{
    QueryBudget, QueryReadBudget, TruncationReason, measure_json_bounded, query_deadline,
};
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

// 仅由固定 revision 的 active 批次生成阻止项；dependency_only 只解析实体来源。
const TYPED_BLOCKED_SEED: &str = "
    UNION
    SELECT json_extract(json_extract(e.entity_json, '$.identity'), '$.node_id')
    FROM revision_runs rr
    JOIN collector_runs cr ON cr.run_id=rr.run_id AND cr.snapshot_id=?1
    JOIN relation_run_memberships m ON m.run_id=rr.run_id AND m.snapshot_id=?1
    JOIN relations r ON r.snapshot_id=m.snapshot_id AND r.edge_id=m.edge_id
    JOIN entities e ON e.snapshot_id=r.snapshot_id AND e.entity_id=r.source_entity_id
    WHERE rr.revision_id=?2 AND rr.role='active'
      AND r.relation IN ('used_by_process','protected_by') AND e.kind='resource'
      AND EXISTS(SELECT 1 FROM entity_run_memberships em
          JOIN revision_runs er ON er.run_id=em.run_id AND er.revision_id=?2
          JOIN collector_runs ec ON ec.run_id=em.run_id AND ec.snapshot_id=em.snapshot_id
          WHERE em.snapshot_id=e.snapshot_id AND em.entity_id=e.entity_id
            AND er.role IN ('active','dependency_only'))
";

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
        let deadline =
            query_deadline(budget).map_err(|error| StoreError::InvalidGraph(error.to_string()))?;
        self.candidate_selection_until(snapshot_id, target_bytes, budget, deadline)
    }

    /// 共用请求期限选择候选，节点与全部必需证据整体提交。
    /// 参数：snapshot_id/target_bytes/budget 保持旧契约，deadline 在首次准备前产生。
    /// 返回：完整候选前缀与精确目标缺口；真实格式错误传播，预算不伪造可用候选。
    pub fn candidate_selection_until(
        &self,
        snapshot_id: &str,
        target_bytes: u64,
        budget: QueryBudget,
        deadline: Instant,
    ) -> Result<CandidateSelection> {
        self.candidate_selection_query(snapshot_id, None, target_bytes, budget, deadline)
    }

    /// 按固定 revision 的有效证据选择审阅候选，保留旧静态证据与预算语义。
    /// 参数：revision_id 为已授权版本，其余参数为目标和整个请求的共享预算。
    /// 返回：有界候选；有效占用/保护排除节点及祖先和后代，歧义旧来源要求重新索引。
    pub fn candidate_selection_for_revision_until(
        &self,
        revision_id: &str,
        target_bytes: u64,
        budget: QueryBudget,
        deadline: Instant,
    ) -> Result<CandidateSelection> {
        let evidence = self.revision_evidence(revision_id)?;
        evidence.require_confirmed_membership()?;
        self.candidate_selection_query(
            evidence.snapshot_id(),
            Some(revision_id),
            target_bytes,
            budget,
            deadline,
        )
    }

    fn candidate_selection_query(
        &self,
        snapshot_id: &str,
        revision_id: Option<&str>,
        target_bytes: u64,
        budget: QueryBudget,
        deadline: Instant,
    ) -> Result<CandidateSelection> {
        let mut reads = QueryReadBudget::new(budget, deadline)
            .map_err(|error| StoreError::InvalidGraph(error.to_string()))?;
        let coverage_complete = self.snapshot(snapshot_id)?.coverage.complete;
        let mut result = CandidateSelection::empty(target_bytes, coverage_complete);
        if !reads.check() {
            result.stop(TruncationReason::Deadline);
            return bounded_selection(result, budget);
        }
        if target_bytes == 0 || !coverage_complete {
            return bounded_selection(result, budget);
        }
        let sql = if revision_id.is_some() {
            CANDIDATE_SQL.replacen(
                "\n),\nblocked_up",
                &format!("{TYPED_BLOCKED_SEED}\n),\nblocked_up"),
                1,
            )
        } else {
            CANDIDATE_SQL.to_owned()
        };
        let mut statement = self.connection.prepare(&sql)?;
        let rows = if let Some(revision) = revision_id {
            statement.query(params![snapshot_id, revision])
        } else {
            statement.query(params![snapshot_id])
        };
        let mut rows = match rows {
            Ok(rows) => rows,
            Err(error) if interrupted(&error) => {
                result.stop(TruncationReason::Deadline);
                return bounded_selection(result, budget);
            }
            Err(error) => return Err(error.into()),
        };
        let mut parent_statement = self
            .connection
            .prepare("SELECT parent_id FROM nodes WHERE snapshot_id = ?1 AND id = ?2")?;
        let mut selected = HashSet::new();
        let mut selected_lineage = HashSet::new();
        // 预留诊断/envelope，末段仍对实际 typed 数据精确计量。raw 账本独立。
        let response_cap = budget.max_response_bytes.saturating_sub(2048);
        let mut response_bytes = 0usize;
        loop {
            if !reads.check() {
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
            if !reads.admit(0, 0, 24) {
                result.stop(reads.stopped().unwrap_or(TruncationReason::ByteLimit));
                break;
            }
            let id =
                u64::try_from(row.get::<_, i64>(0)?).map_err(|_| StoreError::IntegerOverflow)?;
            let mut parent = row.get::<_, Option<i64>>(1)?;
            let bytes =
                u64::try_from(row.get::<_, i64>(2)?).map_err(|_| StoreError::IntegerOverflow)?;
            if selected_lineage.contains(&id) {
                continue;
            }
            let mut lineage = Vec::new();
            let mut visited = HashSet::new();
            let mut overlaps = false;
            while let Some(parent_id) = parent {
                if !reads.check() {
                    result.stop(TruncationReason::Deadline);
                    return bounded_selection(result, budget);
                }
                let parent_id_u64 =
                    u64::try_from(parent_id).map_err(|_| StoreError::IntegerOverflow)?;
                if !visited.insert(parent_id_u64) {
                    return Err(StoreError::InvalidGraph("candidate parent cycle".into()));
                }
                if selected.contains(&parent_id_u64) {
                    overlaps = true;
                    break;
                }
                if !reads.admit(0, 0, 16) {
                    result.stop(reads.stopped().unwrap_or(TruncationReason::ByteLimit));
                    return bounded_selection(result, budget);
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
                        return bounded_selection(result, budget);
                    }
                    Err(error) => return Err(error.into()),
                };
            }
            if overlaps {
                continue;
            }
            let node = match self.node_with_budget(snapshot_id, id, &mut reads) {
                Ok(Some(node)) => node,
                Ok(None) => {
                    return Err(StoreError::InvalidGraph("candidate node is missing".into()));
                }
                Err(StoreError::BudgetExceeded) => {
                    result.stop(reads.stopped().unwrap_or(TruncationReason::ByteLimit));
                    break;
                }
                Err(error) if error.is_interrupted() => {
                    result.stop(TruncationReason::Deadline);
                    break;
                }
                Err(error) => return Err(error),
            };
            let evidence = match self.evidence_with_budget(snapshot_id, id, &mut reads) {
                Ok(evidence) => evidence,
                Err(StoreError::BudgetExceeded) => {
                    result.stop(reads.stopped().unwrap_or(TruncationReason::ByteLimit));
                    break;
                }
                Err(error) if error.is_interrupted() => {
                    result.stop(TruncationReason::Deadline);
                    break;
                }
                Err(error) => return Err(error),
            };
            let Some(charge) = measure_json_bounded(
                &(&node, &evidence),
                response_cap.saturating_sub(response_bytes),
            )?
            else {
                result.stop(TruncationReason::ByteLimit);
                break;
            };
            // 只有完整节点及必需证据准入成功才更新选择与空间目标，逗号按实际逐项预留。
            let Some(total) = response_bytes
                .checked_add(charge)
                .and_then(|n| n.checked_add(1))
                .filter(|n| *n <= response_cap)
            else {
                result.stop(TruncationReason::ByteLimit);
                break;
            };
            if !reads.check() {
                result.stop(TruncationReason::Deadline);
                break;
            }
            response_bytes = total;
            result.selected_bytes = result
                .selected_bytes
                .checked_add(bytes)
                .ok_or(StoreError::IntegerOverflow)?;
            result.remaining_bytes = target_bytes.saturating_sub(result.selected_bytes);
            result.candidates.push((node, evidence));
            selected.insert(id);
            selected_lineage.insert(id);
            selected_lineage.extend(lineage);
            if result.remaining_bytes == 0 {
                break;
            }
        }
        if !reads.check() && reads.stopped() == Some(TruncationReason::Deadline) {
            result.stop(TruncationReason::Deadline);
        }
        bounded_selection(result, budget)
    }
}

fn bounded_selection(
    result: CandidateSelection,
    budget: QueryBudget,
) -> Result<CandidateSelection> {
    if measure_json_bounded(&result, budget.max_response_bytes)?.is_none() {
        return Err(StoreError::BudgetExceeded);
    }
    Ok(result)
}
