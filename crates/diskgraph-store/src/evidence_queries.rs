//! 旧版本节点证据的兼容查询。

use crate::node_codec::as_i64;
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::EvidenceEdge;
use rusqlite::params;
use serde_json::from_str;

impl SqliteSnapshotStore {
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
