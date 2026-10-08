//! 发布事务期间暂停自动 checkpoint；提交后的显式维护由原调用者负责。
use crate::Result;
use rusqlite::Connection;

/// 保存并恢复连接原配置，避免提交在控制 fence 内触发自动 WAL 维护。
/// 来源：DiskGraph 原生 Rust 发布事务与 WAL 生命周期设计。
/// 仅借用原写连接；事务、回执和持久性设置仍由发布入口管理。
pub(crate) struct PublicationCheckpointGuard<'a> {
    connection: &'a Connection,
    previous: i64,
}

impl<'a> PublicationCheckpointGuard<'a> {
    /// 参数：原图库写连接；返回：暂停自动 checkpoint 的守卫或配置失败。
    pub(crate) fn suspend(connection: &'a Connection) -> Result<Self> {
        let previous = connection.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))?;
        connection.pragma_update(None, "wal_autocheckpoint", 0)?;
        Ok(Self {
            connection,
            previous,
        })
    }
}

impl Drop for PublicationCheckpointGuard<'_> {
    fn drop(&mut self) {
        // 成功、回滚和 panic 均恢复；恢复配置本身不执行 checkpoint。
        // 恢复失败不能伪装成发布未提交，也不能在 unwind 中二次 panic。
        if self
            .connection
            .pragma_update(None, "wal_autocheckpoint", self.previous)
            .is_err()
        {
            eprintln!("diskgraph: publication automatic checkpoint configuration restore failed");
        }
    }
}
