//! 可信快照入口的采集批次事务和兼容运行绑定。

use crate::collector_batch_writer::{bind_runs, write_batch};
use crate::{Result, SqliteSnapshotStore, StoreError};
use rusqlite::OptionalExtension;

impl SqliteSnapshotStore {
    /// 事务保存单一快照的采集批次，校验来源并记录成员关系。
    /// 参数：快照、采集运行及本批次实体、证据、关系；不会发布 revision。
    /// 返回：全部写入成功或整体回滚后的错误。
    pub fn record_collector_batch(
        &mut self,
        snapshot_id: &str,
        run: &diskgraph_core::CollectorRun,
        entities: &[diskgraph_core::Entity],
        evidence: &[diskgraph_core::EvidenceRecord],
        edges: &[diskgraph_core::RelationEdge],
    ) -> Result<()> {
        self.snapshot(snapshot_id)?;
        let tx = self.connection.transaction()?;
        write_batch(&tx, snapshot_id, run, entities, evidence, edges)?;
        tx.commit()?;
        Ok(())
    }

    /// 兼容可信内部调用的绑定入口；对外发布使用 publish_collector_revision。
    /// 参数：已有 revision 及运行/角色集合，只允许同快照 active/dependency_only。
    /// 返回：未封存版本绑定或相同绑定的幂等成功；封存版本拒绝新增及角色修改。
    pub fn bind_runs_to_revision(
        &mut self,
        revision_id: &str,
        runs: &[(&str, &str)],
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let snapshot: String = tx
            .query_row(
                "SELECT snapshot_id FROM graph_revisions WHERE revision_id=?1",
                [revision_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::RevisionNotFound(revision_id.into()))?;
        bind_runs(&tx, revision_id, &snapshot, runs)?;
        tx.commit()?;
        Ok(())
    }
}
