//! 历史 pin、保护引用与显式回收。

use crate::{Result, RevisionRecord, SqliteSnapshotStore, StoreError};
use rusqlite::params;

impl SqliteSnapshotStore {
    /// Marks or clears the retention pin on a snapshot.
    /// 执行显式历史保护与回收，保留 pin、最新和引用约束。
    /// 参数：snapshot_id：固定快照 ID；pinned：是否保护历史快照。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn pin_snapshot(&self, snapshot_id: &str, pinned: bool) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE snapshots SET pinned = ?2 WHERE id = ?1",
            params![snapshot_id, pinned as i64],
        )?;
        if changed == 0 {
            return Err(StoreError::SnapshotNotFound(snapshot_id.to_owned()));
        }
        Ok(())
    }

    /// Whether a snapshot is pinned against retention.
    /// 执行显式历史保护与回收，保留 pin、最新和引用约束。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：指定条件是否成立；数据库失败返回错误。
    pub fn snapshot_pinned(&self, snapshot_id: &str) -> Result<bool> {
        self.snapshot(snapshot_id)?;
        let pinned: i64 = self.connection.query_row(
            "SELECT pinned FROM snapshots WHERE id = ?1",
            [snapshot_id],
            |row| row.get(0),
        )?;
        Ok(pinned != 0)
    }

    /// Removes a snapshot from graph history. Refused when pinned, referenced
    /// by a published revision, or backing the current latest pointer
    /// (spec ST-04); never touches user files or control data.
    /// 执行显式历史保护与回收，保留 pin、最新和引用约束。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn remove_snapshot(&mut self, snapshot_id: &str) -> Result<()> {
        if self.snapshot_pinned(snapshot_id)? {
            return Err(StoreError::RetentionViolation(format!(
                "snapshot {snapshot_id} is pinned"
            )));
        }
        let references: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM graph_revisions WHERE snapshot_id = ?1",
            [snapshot_id],
            |row| row.get(0),
        )?;
        if references > 0 {
            return Err(StoreError::RetentionViolation(format!(
                "snapshot {snapshot_id} backs {references} published revision(s)"
            )));
        }
        let changed = self
            .connection
            .execute("DELETE FROM snapshots WHERE id = ?1", [snapshot_id])?;
        if changed == 0 {
            return Err(StoreError::SnapshotNotFound(snapshot_id.to_owned()));
        }
        Ok(())
    }

    /// 显式回收 scope 的旧 revision；默认预览，始终保留最新指针和 pin。
    /// 控制库中的操作/恢复引用保护由 Engine 在控制库写事务内执行。
    /// 执行显式历史保护与回收，保留 pin、最新和引用约束。
    /// 参数：server_id：所属服务器 ID；scope_id：实际所属范围 ID；keep_last：保留最近历史数量；apply：false 只预览，true 持久执行回收。
    /// 返回：`Result<Vec<RevisionRecord>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn prune_revisions(
        &mut self,
        server_id: &str,
        scope_id: &str,
        keep_last: u64,
        apply: bool,
    ) -> Result<Vec<RevisionRecord>> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let revisions: Vec<(RevisionRecord, bool, bool)> = {
            let mut stmt = tx.prepare("SELECT r.revision_id, r.snapshot_id, r.published_at_unix_ms, s.pinned, EXISTS(SELECT 1 FROM latest_revision l WHERE l.revision_id = r.revision_id) FROM graph_revisions r JOIN revision_ownership o ON o.revision_id = r.revision_id JOIN snapshots s ON s.id = r.snapshot_id WHERE o.server_id = ?1 AND o.scope_id = ?2 ORDER BY r.published_at_unix_ms DESC, r.revision_id DESC")?;
            stmt.query_map(params![server_id, scope_id], |row| {
                Ok((
                    RevisionRecord {
                        revision_id: row.get(0)?,
                        snapshot_id: row.get(1)?,
                        published_at_unix_ms: row.get::<_, i64>(2)?.max(0) as u64,
                    },
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?
        };
        let candidates = revisions
            .into_iter()
            .enumerate()
            .filter_map(|(index, (record, pinned, latest))| {
                (index as u64 >= keep_last.max(1) && !pinned && !latest).then_some(record)
            })
            .collect::<Vec<_>>();
        if apply {
            for record in &candidates {
                tx.execute(
                    "DELETE FROM revision_ownership WHERE revision_id = ?1",
                    [&record.revision_id],
                )?;
                tx.execute(
                    "DELETE FROM graph_revisions WHERE revision_id = ?1",
                    [&record.revision_id],
                )?;
                let remaining: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM graph_revisions WHERE snapshot_id = ?1",
                    [&record.snapshot_id],
                    |row| row.get(0),
                )?;
                if remaining == 0 {
                    for table in [
                        "relations",
                        "evidence_records",
                        "entities",
                        "collector_runs",
                    ] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE snapshot_id = ?1"),
                            [&record.snapshot_id],
                        )?;
                    }
                    tx.execute("DELETE FROM snapshots WHERE id = ?1", [&record.snapshot_id])?;
                }
            }
        }
        tx.commit()?;
        Ok(candidates)
    }
}
