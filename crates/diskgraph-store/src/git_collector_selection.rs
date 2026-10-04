//! 固定基线的有界完整运行选择；来源：原生 Rust EV-05 / EC-02。
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{CollectorRun, QueryReadBudget};
use rusqlite::{Connection, OptionalExtension, params};

impl SqliteSnapshotStore {
    /// 参数：固定基线、真实目标 node、共享解码账本；返回：完整运行选择或预算/来源错误，不发布局部集合。
    /// 同目标旧 Git 只保留来源，不重放断言；其余目标与生态的 active 保持原值。
    pub fn git_collector_selection_with_budget(
        &self,
        base_revision: &str,
        node_id: u64,
        reads: &mut QueryReadBudget,
    ) -> Result<Vec<(String, String)>> {
        read(&self.connection, base_revision, node_id, reads, &mut || {
            Ok(())
        })
    }
}
/// 参数：当前图库连接、固定目标、原始账本与运行检查；返回：已确认且有限的完整选择。
pub(crate) fn read(
    connection: &Connection,
    base: &str,
    node: u64,
    reads: &mut QueryReadBudget,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Vec<(String, String)>> {
    check()?;
    if !reads.check() {
        return Err(StoreError::BudgetExceeded);
    }
    let complete:Option<bool>=connection.query_row("SELECT selection_sealed=1 AND evidence_complete=1 FROM graph_revisions WHERE revision_id=?1",
        [base],|row|row.get(0)).optional()?;
    match complete {
        Some(true) => {}
        Some(false) => {
            return Err(StoreError::InvalidGraph(
                "unconfirmed base collector sources; recollect".into(),
            ));
        }
        None => return Err(StoreError::RevisionNotFound(base.into())),
    }
    // 保留损坏历史中缺失的所选 run，NULL 在拥有字段前由原 raw 类型门禁拒绝。
    let mut statement=connection.prepare("SELECT rr.run_id,rr.role,cr.snapshot_id,cr.collector_id,cr.run_json,
        rev.snapshot_id, p.node_id FROM revision_runs rr JOIN graph_revisions rev ON rev.revision_id=rr.revision_id
        LEFT JOIN collector_runs cr ON cr.run_id=rr.run_id LEFT JOIN job_publication_receipts p ON p.run_id=cr.run_id
        WHERE rr.revision_id=?1 ORDER BY rr.run_id LIMIT ?2")?;
    let limit = reads.remaining_edges().min(4096);
    let mut rows = statement.query(params![base, limit as i64])?;
    let mut selected = Vec::new();
    while let Some(row) = rows.next()? {
        check()?;
        let bytes = (0..6).try_fold(0_usize, |total, column| -> Result<usize> {
            total
                .checked_add(
                    row.get_ref(column)?
                        .as_bytes()
                        .map_err(|_| {
                            StoreError::InvalidGraph("invalid run selection field".into())
                        })?
                        .len(),
                )
                .ok_or(StoreError::IntegerOverflow)
        })?;
        if !reads.admit(0, 1, bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        let run: CollectorRun = serde_json::from_str(
            row.get_ref(4)?
                .as_str()
                .map_err(|_| StoreError::InvalidGraph("invalid run payload".into()))?,
        )?;
        let id: String = row.get(0)?;
        let mut role: String = row.get(1)?;
        let snapshot: String = row.get(2)?;
        let collector: String = row.get(3)?;
        if run.run_id != id
            || run.snapshot_id != snapshot
            || snapshot != row.get::<_, String>(5)?
            || run.collector_id != collector
            || !matches!(role.as_str(), "active" | "dependency_only")
        {
            return Err(StoreError::InvalidGraph(
                "invalid selected collector source".into(),
            ));
        }
        if collector == "git-local" {
            let previous_node: Option<i64> = row.get(6)?;
            let Some(previous_node) = previous_node.filter(|n| *n > 0) else {
                return Err(StoreError::InvalidGraph(
                    "Git run has no confirmed target receipt; recollect".into(),
                ));
            };
            if previous_node as u64 == node {
                role = "dependency_only".into();
            }
        }
        selected.push((id, role));
    }
    drop(rows);
    check()?;
    if !reads.check() {
        return Err(StoreError::BudgetExceeded);
    }
    // 页外只 SELECT 常量，不把无法选择的巨 run_json 作为 lookahead 输入。
    let more:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM revision_runs WHERE revision_id=?1 ORDER BY run_id LIMIT 1 OFFSET ?2)",
        params![base,limit as i64],|row|row.get(0))?;
    if more {
        return Err(StoreError::BudgetExceeded);
    }
    check()?;
    Ok(selected)
}
