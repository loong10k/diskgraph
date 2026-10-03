//! 复用文件快照的采集 revision 原子发布。

use crate::collector_batch_writer::{bind_runs, write_batch};
use crate::node_codec::as_i64;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::CollectorBatch;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl SqliteSnapshotStore {
    /// 将采集批次和完整运行选择发布为新 revision，复用基线的文件快照。
    /// 参数：基线与新 revision ID、发布时间、必须匹配的实际 server/scope、批次及 active/dependency_only 运行集合。
    /// 返回：成功时整体提交；归属、来源、选择或数据库错误整体回滚，不改写基线。
    pub fn publish_collector_revision(
        &mut self,
        base_revision_id: &str,
        revision_id: &str,
        published_at_unix_ms: u64,
        expected_owner: (&str, &str),
        batch: &CollectorBatch,
        runs: &[(&str, &str)],
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let base: Option<(String,String,String,String)> = tx.query_row(
            "SELECT r.snapshot_id,s.root_key,o.server_id,o.scope_id FROM graph_revisions r JOIN snapshots s ON s.id=r.snapshot_id JOIN revision_ownership o ON o.revision_id=r.revision_id WHERE r.revision_id=?1",
            [base_revision_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).optional()?;
        let Some((snapshot, root, server, scope)) = base else {
            return Err(StoreError::RevisionNotFound(base_revision_id.into()));
        };
        if (server.as_str(), scope.as_str()) != expected_owner {
            return Err(StoreError::InvalidGraph(
                "base revision ownership mismatch".into(),
            ));
        }
        let latest: String = tx.query_row(
            "SELECT revision_id FROM latest_revision WHERE root_key=?1",
            [&root],
            |row| row.get(0),
        )?;
        if latest != base_revision_id {
            return Err(StoreError::InvalidGraph(
                "stale collector base; refresh from latest revision".into(),
            ));
        }
        if !runs
            .iter()
            .any(|&(id, role)| id == batch.run.run_id && role == "active")
        {
            return Err(StoreError::InvalidGraph(
                "new collector run must be active in published selection".into(),
            ));
        }
        if batch
            .entities
            .iter()
            .any(|entity| !runs.iter().any(|&(id, _)| id == entity.source_run_id))
        {
            return Err(StoreError::InvalidGraph(
                "referenced entity source run must be explicitly selected".into(),
            ));
        }
        write_batch(
            &tx,
            &snapshot,
            &batch.run,
            &batch.entities,
            &batch.evidence,
            &batch.edges,
        )?;
        tx.execute(
            "INSERT INTO graph_revisions (revision_id,snapshot_id,published_at_unix_ms,writer_generation,locator_writer_generation,native_observation_writer_generation) VALUES (?1,?2,?3,10,11,12)",
            params![revision_id, snapshot, as_i64(published_at_unix_ms)?],
        )?;
        tx.execute(
            "INSERT INTO revision_ownership VALUES (?1,?2,?3)",
            params![revision_id, server, scope],
        )?;
        bind_runs(&tx, revision_id, &snapshot, runs)?;
        crate::collector_protocol::seal(&tx, revision_id)?;
        tx.execute("INSERT INTO latest_revision VALUES (?1,?2) ON CONFLICT(root_key) DO UPDATE SET revision_id=excluded.revision_id",params![root,revision_id])?;
        tx.commit()?;
        Ok(())
    }
}
