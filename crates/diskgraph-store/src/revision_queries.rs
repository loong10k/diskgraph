//! revision 归属、查询与旧数据回填。

use rusqlite::OptionalExtension;

use crate::node_codec::as_i64;
use crate::{Result, RevisionRecord, SqliteSnapshotStore, StoreError};
use diskgraph_core::{DiskGraph, DiskSnapshot, ResourceLocator};
use rusqlite::params;
use serde_json::{from_str, to_string};

impl SqliteSnapshotStore {
    /// 纯查询 revision 当前过滤归属是否仍匹配，不拥有持久字符串字段。
    /// 参数：revision_id/server_id/scope_id 是首次准入的真实资源身份。
    /// 返回：未绑定、隔离或归属不符为 false；SQL 错误保留，不构成独立授权。
    /// 来源：DiskGraph 原生 Rust SC-04；无 Java 对应对象。
    pub fn revision_ownership_matches(
        &self,
        revision_id: &str,
        server_id: &str,
        scope_id: &str,
    ) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM revision_authorized_ownership WHERE revision_id=?1 AND server_id=?2 AND scope_id=?3)",
            params![revision_id, server_id, scope_id],
            |row| row.get(0),
        )?)
    }

    /// 返回持久化的实际归属；旧记录未绑定时返回 None，调用者必须拒绝对外访问。
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：revision_id：已发布 revision ID。
    /// 返回：`Result<Option<(String, String)>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn revision_ownership(&self, revision_id: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .connection
            .query_row(
                "SELECT server_id, scope_id FROM revision_authorized_ownership WHERE revision_id = ?1",
                [revision_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// 仅在根定位唯一匹配时回填旧 revision；已绑定记录不覆盖。
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：server_id：所属服务器 ID；roots：注册范围的无损根定位候选，仅唯一匹配允许回填。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn backfill_revision_ownership(
        &mut self,
        server_id: &str,
        roots: &[(String, ResourceLocator)],
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let rows: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT r.revision_id, s.root_key FROM graph_revisions r JOIN snapshots s ON s.id = r.snapshot_id LEFT JOIN revision_ownership o ON o.revision_id = r.revision_id WHERE o.revision_id IS NULL")?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        for (revision, root_key) in rows {
            let root: ResourceLocator = from_str(&root_key)?;
            let matches: Vec<_> = roots
                .iter()
                .filter(|(_, registered)| registered == &root)
                .collect();
            if matches.len() == 1 {
                tx.execute(
                    "INSERT INTO revision_ownership VALUES (?1, ?2, ?3)",
                    params![revision, server_id, matches[0].0],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 将旧 snapshot 标识解析到已绑定归属的 revision，未绑定数据返回 None。
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：`Result<Option<String>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn revision_for_snapshot(&self, snapshot_id: &str) -> Result<Option<String>> {
        Ok(self.connection.query_row("SELECT r.revision_id FROM graph_revisions r JOIN revision_authorized_ownership o ON o.revision_id = r.revision_id WHERE r.snapshot_id = ?1 ORDER BY r.published_at_unix_ms DESC, r.revision_id DESC LIMIT 1", [snapshot_id], |row| row.get(0)).optional()?)
    }

    /// One published revision.
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：revision_id：已发布 revision ID。
    /// 返回：`Result<RevisionRecord>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn revision(&self, revision_id: &str) -> Result<RevisionRecord> {
        self.connection
            .query_row(
                "SELECT snapshot_id, published_at_unix_ms FROM graph_revisions WHERE revision_id = ?1",
                [revision_id],
                |row| {
                    Ok(RevisionRecord {
                        revision_id: revision_id.to_owned(),
                        snapshot_id: row.get(0)?,
                        published_at_unix_ms: row.get::<_, i64>(1)?.try_into().unwrap_or(0),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::RevisionNotFound(revision_id.to_owned()))
    }

    /// The latest published revision for a root locator, if any.
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：root：无损根定位或根过滤条件。
    /// 返回：`Result<Option<String>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn latest_revision_for_root(&self, root: &ResourceLocator) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT lr.revision_id FROM latest_revision lr WHERE lr.root_key = ?1",
                [to_string(root)?],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Loads the full v1 graph behind one revision (v1 projections preserved).
    /// 读取指定快照、节点或窄树行，完整加载仅供可信内部使用。
    /// 参数：revision_id：已发布 revision ID。
    /// 返回：`Result<DiskGraph>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn load_revision(&self, revision_id: &str) -> Result<DiskGraph> {
        self.load(&self.revision(revision_id)?.snapshot_id)
    }

    /// 根据持久 server/scope 归属读取最新 revision，不使用显示路径替代隔离键。
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：server：所属服务器 ID；scope：实际所属范围 ID。
    /// 返回：`Result<Option<String>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn latest_revision_for_scope(&self, server: &str, scope: &str) -> Result<Option<String>> {
        Ok(self.connection.query_row("SELECT r.revision_id FROM graph_revisions r JOIN revision_authorized_ownership o ON o.revision_id = r.revision_id WHERE o.server_id = ?1 AND o.scope_id = ?2 ORDER BY r.published_at_unix_ms DESC,r.revision_id DESC LIMIT 1",params![server,scope],|row| row.get(0)).optional()?)
    }

    /// 只返回实际归属当前 scope 的历史快照；未绑定旧历史不进入外部列表。
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：server：所属服务器 ID；scope：实际所属范围 ID；limit：最大页条数；offset：显式跳过条目数。
    /// 返回：`Result<Vec<DiskSnapshot>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn scope_snapshots(
        &self,
        server: &str,
        scope: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<DiskSnapshot>> {
        let mut statement=self.connection.prepare("SELECT s.snapshot_json FROM snapshots s WHERE EXISTS(SELECT 1 FROM graph_revisions r JOIN revision_authorized_ownership o ON o.revision_id = r.revision_id WHERE r.snapshot_id = s.id AND o.server_id = ?1 AND o.scope_id = ?2) ORDER BY s.captured_at_unix_ms DESC,s.id DESC LIMIT ?3 OFFSET ?4")?;
        let rows = statement.query_map(
            params![server, scope, as_i64(limit)?, as_i64(offset)?],
            |row| row.get::<_, String>(0),
        )?;
        rows.map(|row| Ok(from_str(&row?)?)).collect()
    }

    /// Lists snapshots (optionally for one root), newest first, paged.
    /// 按固定历史或持久归属解析 revision；旧归属仅唯一匹配时回填。
    /// 参数：root：无损根定位或根过滤条件；limit：最大页条数；offset：显式跳过条目数。
    /// 返回：`Result<Vec<DiskSnapshot>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn list_snapshots(
        &self,
        root: Option<&ResourceLocator>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<DiskSnapshot>> {
        let (sql, key): (&str, Option<String>) = match root {
            Some(root) => (
                "SELECT snapshot_json FROM snapshots WHERE root_key = ?1
                 ORDER BY captured_at_unix_ms DESC, id DESC LIMIT ?2 OFFSET ?3",
                Some(to_string(root)?),
            ),
            None => (
                "SELECT snapshot_json FROM snapshots
                 ORDER BY captured_at_unix_ms DESC, id DESC LIMIT ?1 OFFSET ?2",
                None,
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let map_row = |row: &rusqlite::Row<'_>| row.get::<_, String>(0);
        let rows = match key {
            Some(key) => {
                statement.query_map(params![key, as_i64(limit)?, as_i64(offset)?], map_row)?
            }
            None => statement.query_map(params![as_i64(limit)?, as_i64(offset)?], map_row)?,
        };
        rows.map(|row| Ok(from_str::<DiskSnapshot>(&row?)?))
            .collect()
    }
}
