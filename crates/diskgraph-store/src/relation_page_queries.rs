//! 借用 JSON 原始字段准入的关系分页，lookahead 不解码。
use crate::node_codec::as_i64;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{
    QueryBudget, QueryReadBudget, Relation, RelationEdge, TruncationReason, query_deadline,
};
use rusqlite::params;

impl SqliteSnapshotStore {
    /// 兼容原有分页签名，页面原始字节在拥有字段前准入。
    /// 参数：snapshot/entity/direction/relation/after 为过滤，limit/max_bytes 为原额度。
    /// 返回：页与更多标志；首条原始字段超限返回 BudgetExceeded。
    #[allow(clippy::too_many_arguments)]
    pub fn edges_filtered_page(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        outgoing: Option<bool>,
        relation: Option<Relation>,
        after_edge_id: Option<&str>,
        limit: u64,
        max_bytes: usize,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        if limit == 0 {
            return Err(StoreError::InvalidGraph(
                "relation page limit must be positive".into(),
            ));
        }
        let budget = QueryBudget {
            max_edges: usize::try_from(limit).map_err(|_| StoreError::IntegerOverflow)?,
            max_response_bytes: max_bytes.max(1),
            deadline_ms: 60_000,
            ..QueryBudget::default()
        };
        let mut reads = QueryReadBudget::new(
            budget,
            query_deadline(budget).map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
        )
        .map_err(|e| StoreError::InvalidGraph(e.to_string()))?;
        self.edges_page_with_raw_limit(
            snapshot_id,
            entity_id,
            outgoing,
            relation,
            after_edge_id,
            limit,
            &mut reads,
            max_bytes,
            false,
        )
    }

    /// 一页关系共用请求累计边数、原始字段字节与绝对期限。
    /// 参数：固定 snapshot/entity 与方向/关系/keyset 过滤；limit 为当前页，reads 为整次账本。
    /// 返回：已准入前缀与更多标志，零剩余额度只探存在；真实预算内 JSON 错误不吞掉。
    #[allow(clippy::too_many_arguments)]
    pub fn edges_with_budget_page(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        outgoing: Option<bool>,
        relation: Option<Relation>,
        after_edge_id: Option<&str>,
        limit: u64,
        reads: &mut QueryReadBudget,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        self.edges_page_with_raw_limit(
            snapshot_id,
            entity_id,
            outgoing,
            relation,
            after_edge_id,
            limit,
            reads,
            usize::MAX,
            false,
        )
    }

    /// 保留旧 impact 邻接页的严格 TEXT 契约，并继承实际读取账本。
    /// 参数：与 edges_with_budget_page 相同的过滤、页限额及整次 reads；raw 门禁先于列类型检查。
    /// 返回：已准入关系前缀与更多标志；预算内 BLOB/格式错误传播，不改变旧 filtered 字节页。
    #[allow(clippy::too_many_arguments)]
    pub fn edges_text_with_budget_page(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        outgoing: Option<bool>,
        relation: Option<Relation>,
        after_edge_id: Option<&str>,
        limit: u64,
        reads: &mut QueryReadBudget,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        self.edges_page_with_raw_limit(
            snapshot_id,
            entity_id,
            outgoing,
            relation,
            after_edge_id,
            limit,
            reads,
            usize::MAX,
            true,
        )
    }

    // 兼容接口允许零 raw 字节；typed 账本仍保留正数预算契约。
    #[allow(clippy::too_many_arguments)]
    fn edges_page_with_raw_limit(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        outgoing: Option<bool>,
        relation: Option<Relation>,
        after_edge_id: Option<&str>,
        limit: u64,
        reads: &mut QueryReadBudget,
        mut raw_remaining: usize,
        strict_text: bool,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let comparison = if after_edge_id.is_some() { ">" } else { ">=" };
        let predicate = |side: &str| {
            format!(
                "snapshot_id = ?1 AND {side} = ?2 AND edge_id {comparison} ?3 AND (?4 IS NULL OR relation = ?4)"
            )
        };
        let sql = match outgoing {
            Some(side) => format!(
                "SELECT edge_json FROM relations WHERE {} ORDER BY edge_id LIMIT ?5",
                predicate(if side {
                    "source_entity_id"
                } else {
                    "target_entity_id"
                })
            ),
            None => format!(
                "SELECT edge_json,edge_id FROM relations WHERE {} UNION ALL SELECT edge_json,edge_id FROM relations WHERE {} AND source_entity_id != ?2 ORDER BY edge_id LIMIT ?5",
                predicate("source_entity_id"),
                predicate("target_entity_id")
            ),
        };
        let mut statement = self.connection.prepare(&sql)?;
        let limit = limit
            .min(u64::try_from(reads.remaining_edges()).map_err(|_| StoreError::IntegerOverflow)?);
        let mut rows = statement.query(params![
            snapshot_id,
            entity_id,
            after_edge_id.unwrap_or(""),
            relation.map(|r| r.wire_name()),
            as_i64(limit.checked_add(1).ok_or(StoreError::IntegerOverflow)?)?
        ])?;
        let mut edges = Vec::new();
        while let Some(row) = rows.next()? {
            if !reads.check() {
                return Ok((edges, true));
            }
            // limit+1 行只检查存在性，不能拥有或解码坏/巨大 payload。
            if edges.len() as u64 >= limit {
                return Ok((edges, true));
            }
            let json = row.get_ref(0)?.as_bytes().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            if json.len() > raw_remaining {
                reads.stop(TruncationReason::ByteLimit);
            }
            if !reads.admit(0, 1, json.len()) {
                if edges.is_empty() {
                    return Err(StoreError::BudgetExceeded);
                }
                return Ok((edges, true));
            }
            raw_remaining -= json.len();
            let edge = if strict_text {
                let value = row.get_ref(0)?;
                let text = value.as_str().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, value.data_type(), Box::new(error))
                })?;
                serde_json::from_str(text)?
            } else {
                serde_json::from_slice(json)?
            };
            edges.push(edge);
        }
        Ok((edges, false))
    }

    /// 单条实体继承整个请求原始字段额度。
    /// 参数：snapshot/entity 为精确ID，reads 为累计账本。
    /// 返回：实体或 None；先准入再反序列化，预算及真实格式错误区分。
    pub fn entity_with_budget(
        &self,
        snapshot: &str,
        entity: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<diskgraph_core::Entity>> {
        let mut statement = self
            .connection
            .prepare("SELECT entity_json FROM entities WHERE snapshot_id=?1 AND entity_id=?2")?;
        let mut rows = statement.query(params![snapshot, entity])?;
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let json = row.get_ref(0)?.as_bytes().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        if !reads.admit(1, 0, json.len()) {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(Some(serde_json::from_slice(json)?))
    }

    /// 对 explain 实际读取的证据记录计入累计边/字节额度。
    /// 参数：snapshot/id 指定记录，reads 与实体/关系共用。
    /// 返回：完整记录或 None；未知记录不伪造，无预算时不拥有字段。
    pub fn evidence_record_with_budget(
        &self,
        snapshot: &str,
        id: &str,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<diskgraph_core::EvidenceRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT evidence_json FROM evidence_records WHERE snapshot_id=?1 AND evidence_id=?2",
        )?;
        let mut rows = statement.query(params![snapshot, id])?;
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        if reads.remaining_edges() == 0 {
            reads.stop(TruncationReason::EdgeLimit);
            return Err(StoreError::BudgetExceeded);
        }
        let json = row.get_ref(0)?.as_bytes().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        if !reads.admit(0, 1, json.len()) {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(Some(serde_json::from_slice(json)?))
    }
}
