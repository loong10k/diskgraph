//! 采集事务共享的来源校验、不可变写入与成员绑定。

use crate::node_codec::as_i64;
use crate::{Result, StoreError};
use diskgraph_core::{CollectorRun, Entity, EvidenceRecord, RelationEdge};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{from_str, to_string};
use std::collections::{HashMap, HashSet};

/// 校验并维护采集批次与版本的事务边界。
/// 参数：连接和标识来自当前已授权的批次或迁移。
/// 返回：成功结果或需整体回滚的存储错误。
pub(crate) fn write_batch(
    tx: &Connection,
    snapshot_id: &str,
    run: &CollectorRun,
    entities: &[Entity],
    evidence: &[EvidenceRecord],
    edges: &[RelationEdge],
) -> Result<()> {
    if run.snapshot_id != snapshot_id || run.run_id.is_empty() {
        return Err(invalid(
            "collector run targets a different snapshot or has empty id",
        ));
    }
    let kinds = entities
        .iter()
        .map(|entity| (entity.entity_id.clone(), entity.kind))
        .collect::<Vec<_>>();
    diskgraph_core::validate_edges(edges, &kinds).map_err(|error| invalid(error.reason))?;
    let mut ids = HashSet::new();
    for record in evidence {
        if record.run_id != run.run_id || !ids.insert(&record.evidence_id) {
            return Err(invalid("evidence provenance does not match collector run"));
        }
    }
    for edge in edges {
        if edge.evidence_refs.is_empty()
            || edge.evidence_refs.iter().any(|(id, _)| !ids.contains(id))
        {
            return Err(invalid("edge requires evidence belonging to this batch"));
        }
    }
    let entities_by_id: HashMap<_, _> = entities
        .iter()
        .map(|entity| (entity.entity_id.as_str(), entity))
        .collect();
    let mut checked_resources = HashSet::new();
    for edge in edges {
        if !matches!(
            edge.relation,
            diskgraph_core::Relation::UsedByProcess | diskgraph_core::Relation::ProtectedBy
        ) || !checked_resources.insert(edge.source_entity_id.as_str())
        {
            continue;
        }
        let source = entities_by_id
            .get(edge.source_entity_id.as_str())
            .ok_or_else(|| invalid("missing occupied resource"))?;
        if !crate::resource_node_identity::confirmed_node(tx, snapshot_id, source)? {
            return Err(invalid(
                "occupied resource node does not exist in this snapshot",
            ));
        }
    }
    tx.execute(
        "INSERT INTO collector_runs (run_id,snapshot_id,collector_id,collector_version,rule_version,observed_at_unix_ms,coverage_complete,run_json,writer_generation) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,10)",
        params![run.run_id,snapshot_id,run.collector_id,run.collector_version,run.rule_version,as_i64(run.observed_at_unix_ms)?,run.coverage_complete,to_string(run)?],
    )?;
    let mut entity_ids = HashSet::new();
    for entity in entities {
        if !entity_ids.insert(&entity.entity_id) {
            return Err(invalid("duplicate entity within collector batch"));
        }
        let existing: Option<String> = tx
            .query_row(
                "SELECT entity_json FROM entities WHERE snapshot_id=?1 AND entity_id=?2",
                params![snapshot_id, entity.entity_id],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            Some(json) => {
                // 身份的显示值和原始来源也属于不可变记录，复用不得覆盖旧 revision。
                if from_str::<Entity>(&json)? != *entity
                    || !valid_run(tx, snapshot_id, &entity.source_run_id)?
                {
                    return Err(invalid(
                        "existing entity conflicts with immutable value or provenance",
                    ));
                }
            }
            None => {
                if entity.source_run_id != run.run_id {
                    return Err(invalid(
                        "new entity provenance does not match collector run",
                    ));
                }
                tx.execute(
                    "INSERT INTO entities VALUES (?1,?2,?3,?4)",
                    params![
                        snapshot_id,
                        entity.entity_id,
                        entity.kind.wire_name(),
                        to_string(entity)?
                    ],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO entity_run_memberships VALUES (?1,?2,?3)",
            params![snapshot_id, run.run_id, entity.entity_id],
        )?;
    }
    for record in evidence {
        tx.execute(
            "INSERT INTO evidence_records VALUES (?1,?2,?3,?4)",
            params![
                snapshot_id,
                record.evidence_id,
                record.run_id,
                to_string(record)?
            ],
        )?;
    }
    for edge in edges {
        tx.execute(
            "INSERT INTO relations VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                snapshot_id,
                edge.edge_id,
                edge.source_entity_id,
                edge.relation.wire_name(),
                edge.target_entity_id,
                to_string(edge)?
            ],
        )?;
        tx.execute(
            "INSERT INTO relation_run_memberships VALUES (?1,?2,?3)",
            params![snapshot_id, run.run_id, edge.edge_id],
        )?;
    }
    Ok(())
}

/// 校验并维护采集批次与版本的事务边界。
/// 参数：连接和标识来自当前已授权的批次或迁移。
/// 返回：成功结果或需整体回滚的存储错误。
pub(crate) fn valid_run(tx: &Connection, snapshot: &str, run_id: &str) -> Result<bool> {
    let json: Option<String> = tx
        .query_row(
            "SELECT run_json FROM collector_runs WHERE snapshot_id=?1 AND run_id=?2",
            params![snapshot, run_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(json
        .and_then(|json| from_str::<CollectorRun>(&json).ok())
        .is_some_and(|run| run.snapshot_id == snapshot && run.run_id == run_id))
}

/// 校验并维护采集批次与版本的事务边界。
/// 参数：连接和标识来自当前已授权的批次或迁移。
/// 返回：成功结果或需整体回滚的存储错误。
pub(crate) fn bind_runs(
    tx: &Connection,
    revision: &str,
    snapshot: &str,
    runs: &[(&str, &str)],
) -> Result<()> {
    let mut ids = HashSet::new();
    for &(run_id, role) in runs {
        if !matches!(role, "active" | "dependency_only")
            || !ids.insert(run_id)
            || !valid_run(tx, snapshot, run_id)?
        {
            return Err(invalid(
                "revision run selection contains an invalid role, duplicate, or foreign run",
            ));
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT role FROM revision_runs WHERE revision_id=?1 AND run_id=?2",
                params![revision, run_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if previous != role {
                return Err(invalid("existing revision run role cannot change"));
            }
        } else {
            tx.execute(
                "INSERT INTO revision_runs VALUES (?1,?2,?3)",
                params![revision, run_id, role],
            )?;
        }
    }
    Ok(())
}

fn invalid(reason: &str) -> StoreError {
    StoreError::InvalidGraph(reason.into())
}
