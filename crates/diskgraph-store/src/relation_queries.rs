//! 有界关系邻接、实体和解释证据查询。

use rusqlite::OptionalExtension;

use crate::node_codec::as_i64;
use crate::{Result, SqliteSnapshotStore, StoreError};
use rusqlite::params;
use serde_json::from_str;
use std::collections::HashSet;

impl SqliteSnapshotStore {
    /// Outgoing edges of one entity, optionally filtered by relation.
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID；entity_id：实体 ID；relation：可选关系类型过滤。
    /// 返回：`Result<Vec<diskgraph_core::RelationEdge>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn edges_from(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
    ) -> Result<Vec<diskgraph_core::RelationEdge>> {
        self.select_edges(snapshot_id, "source_entity_id", entity_id, relation)
    }

    /// Incoming edges of one entity, optionally filtered by relation.
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID；entity_id：实体 ID；relation：可选关系类型过滤。
    /// 返回：`Result<Vec<diskgraph_core::RelationEdge>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn edges_to(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
    ) -> Result<Vec<diskgraph_core::RelationEdge>> {
        self.select_edges(snapshot_id, "target_entity_id", entity_id, relation)
    }

    /// Outgoing edges of one entity, decoded only up to the requested page.
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID；entity_id：实体 ID；after_edge_id：上页实际关系 ID，按固定 edge_id 顺序继续；limit：最大页条数。
    /// 返回：(本页关系, has_more)；true 表示条数预算后仍有后续关系，false 表示已读完；数据库/解码失败返回错误。
    pub fn edges_from_page(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        after_edge_id: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::RelationEdge>, bool)> {
        self.select_edges_page(
            snapshot_id,
            "source_entity_id",
            entity_id,
            after_edge_id,
            limit,
        )
    }

    /// Incoming edges of one entity, decoded only up to the requested page.
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID；entity_id：实体 ID；after_edge_id：上页实际关系 ID，按固定 edge_id 顺序继续；limit：最大页条数。
    /// 返回：(本页关系, has_more)；true 表示条数预算后仍有后续关系，false 表示已读完；数据库/解码失败返回错误。
    pub fn edges_to_page(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        after_edge_id: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::RelationEdge>, bool)> {
        self.select_edges_page(
            snapshot_id,
            "target_entity_id",
            entity_id,
            after_edge_id,
            limit,
        )
    }

    /// 按方向、类型和稳定 edge_id 读取一页，解码前限制原始 JSON 总字节。
    #[allow(clippy::too_many_arguments)] // 方向、过滤条件和两个独立预算属于一次分页请求。
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID；entity_id：实体 ID；outgoing：Some(true) 查询出边、Some(false) 查询入边、None 查询双向；relation：可选关系类型过滤；after_edge_id：上页实际关系 ID，按固定 edge_id 顺序继续；limit：最大页条数；max_bytes：本页累计原始 JSON 字节预算。
    /// 返回：(本页关系, has_more)；true 表示存在后续或因预算未读完，false 表示已读完；首条超字节预算返回 BudgetExceeded。
    pub fn edges_filtered_page(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        outgoing: Option<bool>,
        relation: Option<diskgraph_core::Relation>,
        after_edge_id: Option<&str>,
        limit: u64,
        max_bytes: usize,
    ) -> Result<(Vec<diskgraph_core::RelationEdge>, bool)> {
        if limit == 0 {
            return Err(StoreError::InvalidGraph(
                "relation page limit must be positive".into(),
            ));
        }
        // 不使用双向 OR 谓词：SQLite 可归并两个邻接索引，而无需扫整个 revision。
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
        let mut rows = statement.query(params![
            snapshot_id,
            entity_id,
            after_edge_id.unwrap_or(""),
            relation.map(|r| r.wire_name()),
            as_i64(limit.saturating_add(1))?
        ])?;
        let mut edges = Vec::new();
        let mut bytes = 0usize;
        while let Some(row) = rows.next()? {
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
            if bytes.saturating_add(json.len()) > max_bytes {
                if edges.is_empty() {
                    return Err(StoreError::BudgetExceeded);
                }
                return Ok((edges, true));
            }
            bytes += json.len();
            edges.push(serde_json::from_slice(json)?);
        }
        Ok((edges, false))
    }

    /// 单条证据仅在编码长度不超过剩余预算时解码，避免超大 provenance 分配。
    /// 读取对应证据，有界接口在解码前检查单条成本。
    /// 参数：snapshot_id：固定快照 ID；evidence_id：证据记录 ID；max_bytes：单条记录字节上限。
    /// 返回：`Result<Option<diskgraph_core::EvidenceRecord>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn evidence_record_bounded(
        &self,
        snapshot_id: &str,
        evidence_id: &str,
        max_bytes: usize,
    ) -> Result<Option<diskgraph_core::EvidenceRecord>> {
        let mut statement = self.connection.prepare("SELECT evidence_json FROM evidence_records WHERE snapshot_id = ?1 AND evidence_id = ?2")?;
        let mut rows = statement.query(params![snapshot_id, evidence_id])?;
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
        if json.len() > max_bytes {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(Some(serde_json::from_slice(json)?))
    }

    fn select_edges_page(
        &self,
        snapshot_id: &str,
        side: &str,
        entity_id: &str,
        after_edge_id: Option<&str>,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::RelationEdge>, bool)> {
        if limit == 0 {
            return Err(StoreError::InvalidGraph(
                "relation page limit must be positive".into(),
            ));
        }
        let sql = format!(
            "SELECT edge_json FROM relations WHERE snapshot_id = ?1 AND {side} = ?2
             AND (?3 IS NULL OR edge_id > ?3) ORDER BY edge_id LIMIT ?4"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                snapshot_id,
                entity_id,
                after_edge_id,
                as_i64(limit.saturating_add(1))?
            ],
            |row| row.get::<_, String>(0),
        )?;
        let mut edges = Vec::new();
        let mut more = false;
        for row in rows {
            let json = row?;
            if edges.len() as u64 == limit {
                more = true;
                break;
            }
            edges.push(from_str(&json)?);
        }
        Ok((edges, more))
    }

    fn select_edges(
        &self,
        snapshot_id: &str,
        side: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
    ) -> Result<Vec<diskgraph_core::RelationEdge>> {
        let sql = match relation {
            Some(_) => format!(
                "SELECT edge_json FROM relations
                 WHERE snapshot_id = ?1 AND {side} = ?2 AND relation = ?3 ORDER BY edge_id"
            ),
            None => format!(
                "SELECT edge_json FROM relations
                 WHERE snapshot_id = ?1 AND {side} = ?2 ORDER BY edge_id"
            ),
        };
        let mut statement = self.connection.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| row.get::<_, String>(0);
        let rows = match relation {
            Some(relation) => statement.query_map(
                params![snapshot_id, entity_id, relation.wire_name()],
                map_row,
            )?,
            None => statement.query_map(params![snapshot_id, entity_id], map_row)?,
        };
        let edges: Vec<diskgraph_core::RelationEdge> = rows
            .map(|row| Ok(from_str::<diskgraph_core::RelationEdge>(&row?)?))
            .collect::<Result<Vec<_>>>()?;
        Ok(edges)
    }

    /// 原始 entity JSON 超出预算时先拒绝，不分配 String 或执行反序列化。
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID；entity_id：实体 ID；max_bytes：单条记录字节上限。
    /// 返回：`Result<Option<diskgraph_core::Entity>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn entity_bounded(
        &self,
        snapshot_id: &str,
        entity_id: &str,
        max_bytes: usize,
    ) -> Result<Option<diskgraph_core::Entity>> {
        let mut statement = self
            .connection
            .prepare("SELECT entity_json FROM entities WHERE snapshot_id=?1 AND entity_id=?2")?;
        let mut rows = statement.query(params![snapshot_id, entity_id])?;
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
        if json.len() > max_bytes {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(Some(serde_json::from_slice(json)?))
    }

    /// Every typed edge of one snapshot (bounded by the caller).
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：`Result<Vec<diskgraph_core::RelationEdge>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn all_edges(&self, snapshot_id: &str) -> Result<Vec<diskgraph_core::RelationEdge>> {
        let mut statement = self
            .connection
            .prepare("SELECT edge_json FROM relations WHERE snapshot_id = ?1 ORDER BY edge_id")?;
        let rows = statement.query_map([snapshot_id], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(from_str(&row?)?)).collect()
    }

    /// Loads one entity of a snapshot.
    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：snapshot_id：固定快照 ID；entity_id：实体 ID。
    /// 返回：`Result<Option<diskgraph_core::Entity>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn entity(
        &self,
        snapshot_id: &str,
        entity_id: &str,
    ) -> Result<Option<diskgraph_core::Entity>> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT entity_json FROM entities WHERE snapshot_id = ?1 AND entity_id = ?2",
                params![snapshot_id, entity_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    /// Evidence records referenced by the given edges (explain's provenance).
    /// 读取对应证据，有界接口在解码前检查单条成本。
    /// 参数：snapshot_id：固定快照 ID；edge_ids：需读取证据的精确关系 ID 集合。
    /// 返回：`Result<Vec<diskgraph_core::EvidenceRecord>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn evidence_for_edges(
        &self,
        snapshot_id: &str,
        edge_ids: &[String],
    ) -> Result<Vec<diskgraph_core::EvidenceRecord>> {
        let mut records = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for edge_id in edge_ids {
            let edge_json: String = self
                .connection
                .query_row(
                    "SELECT edge_json FROM relations WHERE snapshot_id = ?1 AND edge_id = ?2",
                    params![snapshot_id, edge_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::InvalidGraph(format!("missing edge {edge_id}")))?;
            let edge: diskgraph_core::RelationEdge = from_str(&edge_json)?;
            for (evidence_id, _) in edge.evidence_refs {
                let json: Option<String> = self
                    .connection
                    .query_row(
                        "SELECT evidence_json FROM evidence_records
                     WHERE snapshot_id = ?1 AND evidence_id = ?2",
                        params![snapshot_id, evidence_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if let Some(json) = json {
                    let record: diskgraph_core::EvidenceRecord = from_str(&json)?;
                    if seen.insert(record.evidence_id.clone()) {
                        records.push(record);
                    }
                }
            }
        }
        Ok(records)
    }

    /// 按对应邻接/实体条件读取记录，分页保持稳定排序与预算。
    /// 参数：sql：内部固定 SQL 模板；snapshot_id：固定快照 ID。
    /// 返回：`Result<Vec<T>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub(crate) fn select_json<T: serde::de::DeserializeOwned + Send>(
        &self,
        sql: &str,
        snapshot_id: &str,
    ) -> Result<Vec<T>> {
        let mut statement = self.connection.prepare(sql)?;
        // Vec<u8> skips the per-row UTF-8 validation and String allocation
        // (measured at ~30s of a 91s multi-million-row load); SIMD parsing
        // then keeps the same serde contract at a fraction of the cost.
        let rows = statement.query_map([snapshot_id], |row| {
            Ok(row.get_ref(0)?.as_bytes()?.to_vec())
        })?;
        // Multi-million-row loads spend most of their budget in JSON parsing
        // (measured on the real 4.3M-row workload): simd-json is ~2x SLOWER
        // than serde_json per call at this document size, so the parser stays
        // serde_json. Two real wins instead: skip per-row UTF-8 validation
        // and String allocation with as_bytes(), and parallelize the CPU-bound
        // parsing across cores in bounded chunks (each chunk keeps insertion
        // order, so the result is identical to sequential).
        use rayon::prelude::{ParallelIterator, ParallelSlice};
        let raw: Vec<Vec<u8>> = rows.collect::<std::result::Result<_, _>>()?;
        let chunks: Vec<Result<Vec<T>>> = raw
            .par_chunks(16_384)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|bytes| serde_json::from_slice(bytes).map_err(StoreError::from))
                    .collect()
            })
            .collect();
        let mut out = Vec::with_capacity(raw.len());
        for chunk in chunks {
            out.append(&mut chunk?);
        }
        Ok(out)
    }
}
