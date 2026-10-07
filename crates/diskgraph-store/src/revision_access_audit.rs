//! 原归属审计与缺失原始根证明的隔离；拒绝标记不覆盖原 server/scope。
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::{LocatorKind, QualifiedLocator};
use rusqlite::{OptionalExtension, params};

impl SqliteSnapshotStore {
    /// 参数：revision 为审计对象；返回：原始持久归属，包括被隔离记录，不用于对外授权。
    /// 可信内部审计接口；CLI/MCP/FFI 必须使用过滤后的 revision_ownership。
    pub fn revision_ownership_for_audit(&self, revision: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .connection
            .query_row(
                "SELECT server_id,scope_id FROM revision_ownership WHERE revision_id=?1",
                [revision],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// 参数：原服务器/范围及无损根证明；返回：新增隔离数，不删除快照或改写原归属。
    /// 用于启动时处理有损显示别名组；缺少原始根、根不一致或多个根均拒绝。
    /// 隔离不可由显示匹配解除，管理员须重新索引生成具有原始证明的新 revision。
    pub fn isolate_unconfirmed_revision_roots(
        &mut self,
        server: &str,
        scope: &str,
        root: Option<&QualifiedLocator>,
    ) -> Result<usize> {
        let kind = root.map_or("", |root| match root.kind {
            LocatorKind::NativePath => "native_path",
            LocatorKind::DocumentUri => "document_uri",
        });
        let encoding = root.map_or("", |root| root.encoding.wire_name());
        let raw = root.map_or(&[][..], |root| root.raw.as_slice());
        Ok(self.connection.execute("INSERT OR IGNORE INTO revision_access_denials(revision_id,server_id,scope_id,reason) SELECT o.revision_id,o.server_id,o.scope_id,'root_identity_unconfirmed' FROM revision_ownership o JOIN graph_revisions r ON r.revision_id=o.revision_id WHERE o.server_id=?1 AND o.scope_id=?2 AND NOT EXISTS(SELECT 1 FROM nodes n WHERE n.snapshot_id=r.snapshot_id AND n.parent_id IS NULL AND n.native_locator_kind=?3 AND n.native_locator_encoding=?4 AND n.native_locator_raw=?5 AND NOT EXISTS(SELECT 1 FROM nodes other WHERE other.snapshot_id=n.snapshot_id AND other.parent_id IS NULL AND other.id!=n.id))", params![server,scope,kind,encoding,raw])?)
    }
}
