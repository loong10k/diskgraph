//! revision 关系查询；仅 active 批次提供断言，精确索引探测不物化批次集合。
use crate::revision_evidence_reader::RevisionEvidenceReader;
use crate::{Result, StoreError};
use diskgraph_core::{EvidenceRecord, Relation, RelationEdge};
use rusqlite::{OptionalExtension, params};
use std::collections::HashSet;

impl RevisionEvidenceReader<'_> {
    /// 读取所选 active 批次全部关系。
    /// 参数：无；返回：按 edge_id 稳定排序且去重的关系，仅供可信有界调用者。
    pub fn all_edges(&self) -> Result<Vec<RelationEdge>> {
        self.select_edges(None, None)
    }

    /// 读取实体的所选出向断言。
    /// 参数：entity 为精确 ID，relation 为可选过滤；返回：按 edge_id 排序的关系。
    pub fn edges_from(
        &self,
        entity: &str,
        relation: Option<Relation>,
    ) -> Result<Vec<RelationEdge>> {
        self.select_edges(Some(("source_entity_id", entity)), relation)
    }

    /// 读取实体的所选入向断言。
    /// 参数：entity 为精确 ID，relation 为可选过滤；返回：按 edge_id 排序的关系。
    pub fn edges_to(&self, entity: &str, relation: Option<Relation>) -> Result<Vec<RelationEdge>> {
        self.select_edges(Some(("target_entity_id", entity)), relation)
    }

    /// 分页读取实体的所选出向关系，lookahead 仅探测存在。
    /// 参数：entity/after 为实体与 keyset，limit 为正数页长；返回：稳定页及更多标志。
    pub fn edges_from_page(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        self.legacy_page(entity, true, after, limit)
    }

    /// 分页读取实体的所选入向关系，lookahead 仅探测存在。
    /// 参数：entity/after 为实体与 keyset，limit 为正数页长；返回：稳定页及更多标志。
    pub fn edges_to_page(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<RelationEdge>, bool)> {
        self.legacy_page(entity, false, after, limit)
    }

    /// 解析所选 active 边引用的证据，保留首次引用顺序并去重。
    /// 参数：edge_ids 为精确关系 ID 列表；返回：所选 active/dependency_only 来源证据。
    pub fn evidence_for_edges(&self, edge_ids: &[String]) -> Result<Vec<EvidenceRecord>> {
        let sql = format!(
            "SELECT r.edge_json FROM relations r WHERE r.snapshot_id=?1 AND r.edge_id=?2 AND {}",
            Self::active_predicate("?3")
        );
        let mut statement = self.store.connection.prepare(&sql)?;
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for id in edge_ids {
            let json: Option<String> = statement
                .query_row(params![self.snapshot_id, id, self.revision_id], |row| {
                    row.get(0)
                })
                .optional()?;
            let edge: RelationEdge =
                serde_json::from_str(&json.ok_or_else(|| {
                    StoreError::InvalidGraph(format!("missing selected edge {id}"))
                })?)?;
            for (id, _) in edge.evidence_refs {
                if seen.insert(id.clone())
                    && let Some(record) = self.evidence_record(&id)?
                {
                    result.push(record);
                }
            }
        }
        Ok(result)
    }

    /// 构建 active 成员存在性条件。
    /// 参数：revision_param 为内部版本占位符。
    /// 返回：固定 SQL 片段。
    pub(crate) fn active_predicate(revision_param: &str) -> String {
        format!("EXISTS(SELECT 1 FROM relation_run_memberships m
        JOIN revision_runs rr ON rr.run_id=m.run_id AND rr.revision_id={revision_param} AND rr.role='active'
        JOIN collector_runs cr ON cr.run_id=m.run_id AND cr.snapshot_id=m.snapshot_id
        WHERE m.snapshot_id=r.snapshot_id AND m.edge_id=r.edge_id)")
    }

    fn select_edges(
        &self,
        side: Option<(&str, &str)>,
        relation: Option<Relation>,
    ) -> Result<Vec<RelationEdge>> {
        let side_predicate = side.map_or_else(
            || "?3 IS NULL".into(),
            |(column, _)| format!("r.{column}=?3"),
        );
        let sql = format!(
            "SELECT r.edge_json FROM relations r WHERE r.snapshot_id=?1 AND {} AND {side_predicate} AND (?4 IS NULL OR r.relation=?4) ORDER BY r.edge_id",
            Self::active_predicate("?2")
        );
        let mut statement = self.store.connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                self.snapshot_id,
                self.revision_id,
                side.map(|(_, id)| id),
                relation.map(|r| r.wire_name())
            ],
            |row| row.get::<_, String>(0),
        )?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
}
