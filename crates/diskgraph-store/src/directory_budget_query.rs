//! 目录页的借用字段准入与存在探针；来源：D26 / Q-02。
use crate::directory_aggregates::KNOWN_SIZE;
use crate::node_codec::as_i64;
use crate::node_row::NodeRow;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{DiskNode, QueryReadBudget, TruncationReason};
use rusqlite::{OptionalExtension, params};

impl SqliteSnapshotStore {
    /// 按已有父索引顺序读取当前页，原始字段在拥有和解码之前累计准入。
    /// 参数：snapshot/parent/offset/limit 为固定页；known_only保留session过滤语义，budget共用请求期限。
    /// 返回：已读取节点、是否还有数据、精确未知大小计数；账本记录预算停止原因。
    pub fn children_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        offset: u64,
        limit: u64,
        known_only: bool,
        budget: &mut QueryReadBudget,
    ) -> Result<(Vec<DiskNode>, bool, u64)> {
        self.snapshot_with_budget(snapshot, budget)?;
        let unknown = self
            .connection
            .query_row(
                "SELECT unknown_count FROM directory_counts WHERE snapshot_id=?1 AND parent_id=?2",
                params![snapshot, as_i64(parent)?],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
            .max(0) as u64;
        let filter = if known_only { KNOWN_SIZE } else { "1=1" };
        // 会话页沿用旧查询的非负大小下界，避免损坏负值经无符号转换成为巨大文件。
        let minimum = if known_only {
            "AND subtree_bytes>=0"
        } else {
            ""
        };
        let sql = format!(
            "SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 AND ({filter}) {minimum} ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?3 OFFSET ?4"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let mut rows = statement.query(params![
            snapshot,
            as_i64(parent)?,
            as_i64(limit.checked_add(1).ok_or(StoreError::IntegerOverflow)?)?,
            as_i64(offset)?
        ])?;
        let mut items = Vec::new();
        let more = loop {
            let row = match rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => break false,
                Err(error) => {
                    let error = StoreError::from(error);
                    if error.is_interrupted() {
                        budget.stop(TruncationReason::Deadline);
                        break true;
                    }
                    return Err(error);
                }
            };
            // 页外行只证明下一页存在，不分配字符串或解析坏 continuation。
            if items.len() as u64 == limit {
                break true;
            }
            if !budget.admit(1, 0, NodeRow::raw_bytes(row)?) {
                break true;
            }
            items.push(NodeRow::from_row(row)?.into_node()?);
        };
        budget.check();
        Ok((items, more, unknown))
    }
}
