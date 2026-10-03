//! 采集版本写入协议与封存门禁；来源：EV-05 / D28。
use crate::{Result, StoreError};
use rusqlite::Connection;

/// 在成员迁移事务内建立写入代次、版本完整性和不可变选择。
/// 参数：tx 为正在升级的图库事务连接。
/// 返回：成功建立门禁，任一 SQL 错误由外层回滚。
pub(crate) fn migrate(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "ALTER TABLE graph_revisions ADD COLUMN writer_generation INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE graph_revisions ADD COLUMN selection_sealed INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE graph_revisions ADD COLUMN evidence_complete INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE collector_runs ADD COLUMN writer_generation INTEGER NOT NULL DEFAULT 0;
         CREATE TABLE revision_runs_v10 (
             revision_id TEXT NOT NULL REFERENCES graph_revisions(revision_id) ON DELETE CASCADE,
             run_id TEXT NOT NULL REFERENCES collector_runs(run_id),
             role TEXT NOT NULL,
             PRIMARY KEY(revision_id,run_id));
         INSERT INTO revision_runs_v10 SELECT revision_id,run_id,role FROM revision_runs;
         DROP TABLE revision_runs;
         ALTER TABLE revision_runs_v10 RENAME TO revision_runs;",
    )?;
    // 先逐版本确认再封存；迁移不能替旧选择补全未选上游。
    {
        let mut statement = tx.prepare("SELECT revision_id, NOT EXISTS(
            SELECT 1 FROM collector_membership_diagnostics d WHERE d.snapshot_id=r.snapshot_id AND (
                (d.member_kind='entity' AND (
                    NOT EXISTS(SELECT 1 FROM entity_run_memberships m WHERE m.snapshot_id=d.snapshot_id AND m.entity_id=d.member_id)
                    OR EXISTS(SELECT 1 FROM entity_run_memberships m JOIN revision_runs rr ON rr.run_id=m.run_id AND rr.revision_id=r.revision_id
                        WHERE m.snapshot_id=d.snapshot_id AND m.entity_id=d.member_id)))
                OR (d.member_kind='relation' AND (
                    NOT EXISTS(SELECT 1 FROM relation_run_memberships m WHERE m.snapshot_id=d.snapshot_id AND m.edge_id=d.member_id)
                    OR EXISTS(SELECT 1 FROM relation_run_memberships m JOIN revision_runs rr ON rr.run_id=m.run_id AND rr.revision_id=r.revision_id
                        WHERE m.snapshot_id=d.snapshot_id AND m.edge_id=d.member_id)))
                OR d.member_kind NOT IN ('entity','relation')))
            FROM graph_revisions r")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let revision: String = row.get(0)?;
            let no_diagnostics: bool = row.get(1)?;
            let complete =
                no_diagnostics && crate::revision_source_validation::confirmed(tx, &revision)?;
            tx.execute("UPDATE graph_revisions SET selection_sealed=1,evidence_complete=?2 WHERE revision_id=?1",
                rusqlite::params![revision, complete])?;
        }
    }
    tx.execute_batch(
        "CREATE TRIGGER revisions_require_collector_writer BEFORE INSERT ON graph_revisions
             WHEN NEW.writer_generation!=10 OR NEW.selection_sealed!=0 OR NEW.evidence_complete!=0
             BEGIN SELECT RAISE(ABORT,'obsolete revision writer; reopen with current DiskGraph'); END;
         CREATE TRIGGER collectors_require_member_writer BEFORE INSERT ON collector_runs
             WHEN NEW.writer_generation!=10
             BEGIN SELECT RAISE(ABORT,'obsolete collector writer; reopen with current DiskGraph'); END;
         CREATE TRIGGER revisions_preserve_seal BEFORE UPDATE ON graph_revisions
             WHEN OLD.selection_sealed=1
             BEGIN SELECT RAISE(ABORT,'published revision is immutable'); END;
         CREATE TRIGGER selected_runs_no_append BEFORE INSERT ON revision_runs
             WHEN EXISTS(SELECT 1 FROM graph_revisions WHERE revision_id=NEW.revision_id AND selection_sealed=1)
             BEGIN SELECT RAISE(ABORT,'published revision selection is sealed'); END;
         CREATE TRIGGER selected_runs_no_rewrite BEFORE UPDATE ON revision_runs
             WHEN EXISTS(SELECT 1 FROM graph_revisions WHERE revision_id IN (OLD.revision_id,NEW.revision_id) AND selection_sealed=1)
             BEGIN SELECT RAISE(ABORT,'published revision selection is sealed'); END;
         CREATE TRIGGER selected_runs_no_remove BEFORE DELETE ON revision_runs
             WHEN EXISTS(SELECT 1 FROM graph_revisions WHERE revision_id=OLD.revision_id AND selection_sealed=1)
             BEGIN SELECT RAISE(ABORT,'published revision selection is sealed'); END;",
    )?;
    Ok(())
}

/// 原子发布事务的最后阶段封存版本及完整批次选择。
/// 参数：tx 为发布事务，revision 为本事务创建的版本。
/// 返回：封存成功，旧版本重复封存由数据库拒绝。
pub(crate) fn seal(tx: &Connection, revision: &str) -> Result<()> {
    // 旧歧义运行不能通过新空批次洗成完整版本；新采集只选已确认来源才能恢复。
    let incomplete: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM revision_runs rr JOIN collector_runs cr ON cr.run_id=rr.run_id
         WHERE rr.revision_id=?1 AND cr.writer_generation!=10
         AND EXISTS(SELECT 1 FROM collector_membership_diagnostics d WHERE d.snapshot_id=cr.snapshot_id))",
        [revision], |row| row.get(0))?;
    if incomplete || !crate::revision_source_validation::confirmed(tx, revision)? {
        return Err(StoreError::InvalidGraph(
            "selected collector sources are incomplete; recollect or include upstream runs".into(),
        ));
    }
    tx.execute(
        "UPDATE graph_revisions SET selection_sealed=1,evidence_complete=1 WHERE revision_id=?1",
        [revision],
    )?;
    Ok(())
}
