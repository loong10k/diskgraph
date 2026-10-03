//! 旧版本节点证据的兼容查询。

use crate::node_codec::as_i64;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{EvidenceEdge, QueryReadBudget};
use rusqlite::params;
use serde_json::from_str;

impl SqliteSnapshotStore {
    /// 全部必需证据按累计边数及原始字段成本准入，超限不返回缺证据候选。
    /// 参数：snapshot/node_id 固定节点，budget 与节点/其他候选共用。
    /// 返回：完整证据或预算/格式错误；额外行仅探存在，不拥有或解码 payload。
    pub fn evidence_with_budget(
        &self,
        snapshot: &str,
        node_id: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<Vec<EvidenceEdge>> {
        let mut statement = self.connection.prepare(
            "SELECT evidence_json FROM evidence WHERE snapshot_id=?1 AND node_id=?2 ORDER BY rowid",
        )?;
        let mut rows = statement.query(params![snapshot, as_i64(node_id)?])?;
        let mut evidence = Vec::new();
        while let Some(row) = rows.next()? {
            if !budget.check() {
                return Err(StoreError::BudgetExceeded);
            }
            // 边额度已用完时不触碰 lookahead 字段，保留真实存在性。
            if budget.remaining_edges() == 0 {
                budget.stop(diskgraph_core::TruncationReason::EdgeLimit);
                return Err(StoreError::BudgetExceeded);
            }
            let json = row.get_ref(0)?.as_bytes().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            if !budget.admit(0, 1, json.len()) {
                return Err(StoreError::BudgetExceeded);
            }
            // 先执行 raw 门禁，再保留旧 TEXT 列契约；BLOB 即便包含合法 JSON 也不能提升为证据。
            let value = row.get_ref(0)?;
            let text = value.as_str().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, value.data_type(), Box::new(error))
            })?;
            evidence.push(serde_json::from_str(text)?);
        }
        Ok(evidence)
    }

    /// 读取对应证据，有界接口在解码前检查单条成本。
    /// 参数：snapshot_id：固定快照 ID；node_id：精确节点 ID。
    /// 返回：`Result<Vec<EvidenceEdge>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn evidence(&self, snapshot_id: &str, node_id: u64) -> Result<Vec<EvidenceEdge>> {
        self.snapshot(snapshot_id)?;
        let mut statement = self.connection.prepare(
            "SELECT evidence_json FROM evidence
             WHERE snapshot_id = ?1 AND node_id = ?2 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![snapshot_id, as_i64(node_id)?], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| Ok(from_str(&row?)?)).collect()
    }
}
