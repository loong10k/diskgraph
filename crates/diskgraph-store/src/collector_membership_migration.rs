//! v9 -> v10：仅从可验证来源恢复采集成员，歧义记录保留重采诊断。

use crate::Result;
use crate::collector_batch_writer::valid_run;
use diskgraph_core::{Entity, EvidenceRecord, Relation, RelationEdge};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::from_str;

/// 校验并维护采集批次与版本的事务边界。
/// 参数：连接和标识来自当前已授权的批次或迁移。
/// 返回：成功结果或需整体回滚的存储错误。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE relation_run_memberships (
            snapshot_id TEXT NOT NULL,
            run_id TEXT NOT NULL REFERENCES collector_runs(run_id) ON DELETE CASCADE,
            edge_id TEXT NOT NULL,
            PRIMARY KEY(snapshot_id,run_id,edge_id),
            FOREIGN KEY(snapshot_id,edge_id) REFERENCES relations(snapshot_id,edge_id) ON DELETE CASCADE
         );
         CREATE INDEX relation_memberships_by_member ON relation_run_memberships(snapshot_id,edge_id,run_id);
         CREATE TABLE entity_run_memberships (
            snapshot_id TEXT NOT NULL,
            run_id TEXT NOT NULL REFERENCES collector_runs(run_id) ON DELETE CASCADE,
            entity_id TEXT NOT NULL,
            PRIMARY KEY(snapshot_id,run_id,entity_id),
            FOREIGN KEY(snapshot_id,entity_id) REFERENCES entities(snapshot_id,entity_id) ON DELETE CASCADE
         );
         CREATE INDEX entity_memberships_by_member ON entity_run_memberships(snapshot_id,entity_id,run_id);
         CREATE TABLE collector_membership_diagnostics (
            snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
            member_kind TEXT NOT NULL,
            member_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            PRIMARY KEY(snapshot_id,member_kind,member_id)
         );",
    )?;
    crate::relation_membership_index::create(&tx)?;
    {
        let mut stmt = tx.prepare("SELECT snapshot_id,entity_id,kind,entity_json FROM entities")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let snapshot: String = row.get(0)?;
            let id: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let json: String = row.get(3)?;
            let entity = from_str::<Entity>(&json).ok();
            let run = if let Some(entity) = entity {
                if entity.entity_id == id
                    && entity.kind.wire_name() == kind
                    && valid_run(&tx, &snapshot, &entity.source_run_id)?
                {
                    Some(entity.source_run_id)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(run) = run {
                tx.execute(
                    "INSERT INTO entity_run_memberships VALUES (?1,?2,?3)",
                    params![snapshot, run, id],
                )?;
            } else {
                diagnostic(&tx, &snapshot, "entity", &id)?;
            }
        }
    }
    {
        let mut stmt = tx.prepare("SELECT snapshot_id,edge_id,source_entity_id,relation,target_entity_id,edge_json FROM relations")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let snapshot: String = row.get(0)?;
            let id: String = row.get(1)?;
            let source: String = row.get(2)?;
            let relation: String = row.get(3)?;
            let target: String = row.get(4)?;
            let json: String = row.get(5)?;
            let run = if let Ok(edge) = from_str::<RelationEdge>(&json) {
                if edge.edge_id == id
                    && edge.source_entity_id == source
                    && edge.target_entity_id == target
                    && edge.relation.wire_name() == relation
                {
                    inferred_run(&tx, &snapshot, &edge)?
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(run) = run {
                tx.execute(
                    "INSERT INTO relation_run_memberships VALUES (?1,?2,?3)",
                    params![snapshot, run, id],
                )?;
            } else {
                diagnostic(&tx, &snapshot, "relation", &id)?;
            }
        }
    }
    // 成员来源可确认但历史 revision 未选择该运行时，禁止猜测绑定并保留重采提示。
    tx.execute_batch("INSERT OR IGNORE INTO collector_membership_diagnostics
        SELECT m.snapshot_id,'entity',m.entity_id,'legacy_revision_selection_unavailable_recollect'
        FROM entity_run_memberships m WHERE NOT EXISTS (
            SELECT 1 FROM revision_runs rr JOIN graph_revisions r ON r.revision_id=rr.revision_id
            WHERE r.snapshot_id=m.snapshot_id AND rr.run_id=m.run_id AND rr.role IN ('active','dependency_only'));
        INSERT OR IGNORE INTO collector_membership_diagnostics
        SELECT m.snapshot_id,'relation',m.edge_id,'legacy_revision_selection_unavailable_recollect'
        FROM relation_run_memberships m WHERE NOT EXISTS (
            SELECT 1 FROM revision_runs rr JOIN graph_revisions r ON r.revision_id=rr.revision_id
            WHERE r.snapshot_id=m.snapshot_id AND rr.run_id=m.run_id AND rr.role IN ('active','dependency_only'));
        PRAGMA user_version=10;")?;
    crate::collector_protocol::migrate(&tx)?;
    tx.commit()?;
    Ok(())
}

fn inferred_run(tx: &Connection, snapshot: &str, edge: &RelationEdge) -> Result<Option<String>> {
    let mut endpoints = Vec::with_capacity(2);
    for id in [&edge.source_entity_id, &edge.target_entity_id] {
        let json: Option<String> = tx
            .query_row(
                "SELECT e.entity_json FROM entities e JOIN entity_run_memberships m
             ON m.snapshot_id=e.snapshot_id AND m.entity_id=e.entity_id
             WHERE e.snapshot_id=?1 AND e.entity_id=?2",
                params![snapshot, id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(entity) = json.and_then(|json| from_str::<Entity>(&json).ok()) else {
            return Ok(None);
        };
        if id == &edge.source_entity_id
            && matches!(
                edge.relation,
                Relation::UsedByProcess | Relation::ProtectedBy
            )
            && !crate::resource_node_identity::confirmed_node(tx, snapshot, &entity)?
        {
            return Ok(None);
        }
        endpoints.push((entity.entity_id, entity.kind));
    }
    if diskgraph_core::validate_edges(std::slice::from_ref(edge), &endpoints).is_err() {
        return Ok(None);
    }
    let mut selected: Option<String> = None;
    for (id, _) in &edge.evidence_refs {
        let record: Option<(String,String)> = tx.query_row("SELECT run_id,evidence_json FROM evidence_records WHERE snapshot_id=?1 AND evidence_id=?2",params![snapshot,id],|row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        let Some((run, json)) = record else {
            return Ok(None);
        };
        let Ok(evidence) = from_str::<EvidenceRecord>(&json) else {
            return Ok(None);
        };
        if evidence.evidence_id != *id
            || evidence.run_id != run
            || !valid_run(tx, snapshot, &run)?
            || selected.as_ref().is_some_and(|previous| previous != &run)
        {
            return Ok(None);
        }
        selected = Some(run);
    }
    Ok(selected)
}

fn diagnostic(tx: &Connection, snapshot: &str, kind: &str, id: &str) -> Result<()> {
    tx.execute("INSERT INTO collector_membership_diagnostics VALUES (?1,?2,?3,'legacy_membership_unavailable_recollect')",params![snapshot,kind,id])?;
    Ok(())
}
