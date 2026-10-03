//! revision 邻接页的预算与严格 TEXT 门禁。
use crate::node_codec::as_i64;
use crate::revision_edge_cursor::RevisionEdgeCursor;
use crate::revision_evidence_reader::RevisionEvidenceReader;
use crate::{Result, StoreError};
use diskgraph_core::{QueryReadBudget, Relation, RelationEdge, TruncationReason};
use rusqlite::params;

impl RevisionEvidenceReader<'_> {
    /// 借用原始字段准入后解码所选 active 关系。
    /// 参数：entity/direction/relation/after 为过滤，limit 为页长，reads 为整个请求账本。
    /// 返回：已准入前缀与更多标志，lookahead 不解码且双向自环仅返回一次。
    #[allow(clippy::too_many_arguments)]
    pub fn edges_with_budget_page(
        &self,
        entity: &str,
        outgoing: Option<bool>,
        relation: Option<Relation>,
        after: Option<&str>,
        limit: u64,
        reads: &mut QueryReadBudget,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        self.budget_page(entity, outgoing, relation, after, limit, reads, false)
    }

    /// 沿用旧 impact 的严格 TEXT 契约；原始字段先计入共享额度。
    /// 参数：entity/direction/relation/after 为过滤，limit 为页长，reads 为共享账本。
    /// 返回：稳定关系前缀及更多标志，预算内 BLOB 类型错误不被隐藏。
    #[allow(clippy::too_many_arguments)]
    pub fn edges_text_with_budget_page(
        &self,
        entity: &str,
        outgoing: Option<bool>,
        relation: Option<Relation>,
        after: Option<&str>,
        limit: u64,
        reads: &mut QueryReadBudget,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        self.budget_page(entity, outgoing, relation, after, limit, reads, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn budget_page(
        &self,
        entity: &str,
        outgoing: Option<bool>,
        relation: Option<Relation>,
        after: Option<&str>,
        limit: u64,
        reads: &mut QueryReadBudget,
        strict_text: bool,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let mut cursor = RevisionEdgeCursor::new(self, entity, outgoing, relation, after, reads)?;
        let limit = limit
            .min(u64::try_from(reads.remaining_edges()).map_err(|_| StoreError::IntegerOverflow)?);
        let mut statement = self
            .store
            .connection
            .prepare("SELECT edge_json FROM relations WHERE snapshot_id=?1 AND edge_id=?2")?;
        let mut edges = Vec::new();
        while let Some(key) = cursor.peek() {
            if !reads.check() {
                return Ok((edges, true));
            }
            if edges.len() as u64 >= limit {
                return Ok((edges, true));
            }
            let mut rows = statement.query(params![self.snapshot_id, key])?;
            let row = rows
                .next()?
                .ok_or_else(|| StoreError::InvalidGraph("selected edge is missing".into()))?;
            let value = row.get_ref(0)?;
            let json = value.as_bytes().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, value.data_type(), Box::new(error))
            })?;
            if !reads.admit(0, 1, json.len()) {
                if edges.is_empty() {
                    return Err(StoreError::BudgetExceeded);
                }
                return Ok((edges, true));
            }
            let edge = if strict_text {
                let text = value.as_str().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, value.data_type(), Box::new(error))
                })?;
                serde_json::from_str(text)?
            } else {
                serde_json::from_slice(json)?
            };
            edges.push(edge);
            match cursor.advance(reads) {
                Ok(()) => {}
                Err(StoreError::BudgetExceeded) => return Ok((edges, true)),
                Err(error) if error.is_interrupted() => {
                    reads.stop(TruncationReason::Deadline);
                    return Ok((edges, true));
                }
                Err(error) => return Err(error),
            }
        }
        if !reads.check() {
            return Ok((edges, true));
        }
        Ok((edges, false))
    }

    /// 保留可信旧分页语义。
    /// 参数：邻接过滤和正数页长。
    /// 返回：关系页及存在性标志。
    pub(crate) fn legacy_page(
        &self,
        entity: &str,
        outgoing: bool,
        after: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        if limit == 0 {
            return Err(StoreError::InvalidGraph(
                "relation page limit must be positive".into(),
            ));
        }
        let sql = Self::page_sql(Some(outgoing), after.is_some());
        let mut statement = self.store.connection.prepare(&sql)?;
        let mut rows = statement.query(params![
            self.snapshot_id,
            entity,
            after.unwrap_or(""),
            Option::<&str>::None,
            as_i64(limit.checked_add(1).ok_or(StoreError::IntegerOverflow)?)?,
            self.revision_id
        ])?;
        let mut edges = Vec::new();
        while let Some(row) = rows.next()? {
            if edges.len() as u64 == limit {
                return Ok((edges, true));
            }
            let value = row.get_ref(0)?;
            let json = value.as_str().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, value.data_type(), Box::new(error))
            })?;
            edges.push(serde_json::from_str(json)?);
        }
        Ok((edges, false))
    }

    fn page_sql(outgoing: Option<bool>, has_after: bool) -> String {
        let comparison = if has_after { ">" } else { ">=" };
        let predicate = |side| {
            format!(
                "r.snapshot_id=?1 AND r.{side}=?2 AND r.edge_id {comparison} ?3 AND (?4 IS NULL OR r.relation=?4) AND {}",
                Self::active_predicate("?6")
            )
        };
        match outgoing {
            Some(side) => format!(
                "SELECT r.edge_json FROM relations r WHERE {} ORDER BY r.edge_id LIMIT ?5",
                predicate(if side {
                    "source_entity_id"
                } else {
                    "target_entity_id"
                })
            ),
            None => format!(
                "SELECT r.edge_json,r.edge_id FROM relations r WHERE {} UNION ALL SELECT r.edge_json,r.edge_id FROM relations r WHERE {} AND r.source_entity_id!=?2 ORDER BY edge_id LIMIT ?5",
                predicate("source_entity_id"),
                predicate("target_entity_id")
            ),
        }
    }
}
