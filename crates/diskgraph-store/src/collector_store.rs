//! 采集批次、实体、证据和关系的持久化事务。

use crate::node_codec::as_i64;
use crate::{Result, SqliteSnapshotStore, StoreError};
use rusqlite::params;
use serde_json::to_string;

impl SqliteSnapshotStore {
    /// Records one collector batch: run, entities, evidence, and validated
    /// typed edges, atomically for one snapshot (EV-01/EV-02). Endpoint
    /// violations reject the whole batch before anything is written.
    /// 事务保存采集批次、实体、证据、关系及 revision 角色绑定。
    /// 参数：snapshot_id：固定快照 ID；run：采集运行记录；entities：实体身份及来源集合；evidence：有来源的证据集合；edges：类型化关系集合。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn record_collector_batch(
        &mut self,
        snapshot_id: &str,
        run: &diskgraph_core::CollectorRun,
        entities: &[diskgraph_core::Entity],
        evidence: &[diskgraph_core::EvidenceRecord],
        edges: &[diskgraph_core::RelationEdge],
    ) -> Result<()> {
        self.snapshot(snapshot_id)?;
        if run.snapshot_id != snapshot_id {
            return Err(StoreError::InvalidGraph(
                "collector run targets a different snapshot".into(),
            ));
        }
        let entity_kinds: Vec<(String, diskgraph_core::EntityKind)> = entities
            .iter()
            .map(|entity| (entity.entity_id.clone(), entity.kind))
            .collect();
        diskgraph_core::validate_edges(edges, &entity_kinds).map_err(|error| {
            StoreError::InvalidGraph(format!(
                "edge {} ({}): {}",
                error.edge_id,
                error.relation.wire_name(),
                error.reason
            ))
        })?;
        if edges.iter().any(|edge| {
            edge.evidence_refs.iter().any(|(evidence_id, _)| {
                !evidence
                    .iter()
                    .any(|record| record.evidence_id == *evidence_id)
            })
        }) {
            return Err(StoreError::InvalidGraph(
                "edge references evidence outside this batch".into(),
            ));
        }
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO collector_runs (run_id, snapshot_id, collector_id, collector_version, rule_version, observed_at_unix_ms, coverage_complete, run_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                run.run_id,
                snapshot_id,
                run.collector_id,
                run.collector_version as i64,
                run.rule_version as i64,
                as_i64(run.observed_at_unix_ms)?,
                run.coverage_complete as i64,
                to_string(run)?,
            ],
        )?;
        {
            let mut entity_statement = transaction.prepare(
                "INSERT INTO entities (snapshot_id, entity_id, kind, entity_json) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for entity in entities {
                entity_statement.execute(params![
                    snapshot_id,
                    entity.entity_id,
                    entity.kind.wire_name(),
                    to_string(entity)?,
                ])?;
            }
            let mut evidence_statement = transaction.prepare(
                "INSERT INTO evidence_records (snapshot_id, evidence_id, run_id, evidence_json) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for record in evidence {
                evidence_statement.execute(params![
                    snapshot_id,
                    record.evidence_id,
                    record.run_id,
                    to_string(record)?,
                ])?;
            }
            let mut edge_statement = transaction.prepare(
                "INSERT INTO relations (snapshot_id, edge_id, source_entity_id, relation, target_entity_id, edge_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for edge in edges {
                edge_statement.execute(params![
                    snapshot_id,
                    edge.edge_id,
                    edge.source_entity_id,
                    edge.relation.wire_name(),
                    edge.target_entity_id,
                    to_string(edge)?,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Binds collector runs to a published revision with their roles
    /// (active vs dependency-only) (EV-05).
    /// 事务保存采集批次、实体、证据、关系及 revision 角色绑定。
    /// 参数：revision_id：已发布 revision ID；runs：采集批次 ID 集合。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn bind_runs_to_revision(
        &mut self,
        revision_id: &str,
        runs: &[(&str, &str)],
    ) -> Result<()> {
        self.revision(revision_id)?;
        let transaction = self.connection.transaction()?;
        {
            let mut statement = transaction.prepare(
                "INSERT OR IGNORE INTO revision_runs (revision_id, run_id, role) VALUES (?1, ?2, ?3)",
            )?;
            for (run_id, role) in runs {
                statement.execute(params![revision_id, run_id, role])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}
