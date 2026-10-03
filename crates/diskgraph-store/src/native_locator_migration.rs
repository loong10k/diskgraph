//! v10 -> v11 无损定位字段和独立发布写入代次。

use crate::Result;
use rusqlite::Connection;

/// 在单个事务中新增可空定位字段与独立发布代次，不推断旧路径字节。
/// 参数：connection 为待从 v10 升级的图库连接。
/// 返回：迁移全部提交或错误；失败时由事务回滚。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(
        "ALTER TABLE nodes ADD COLUMN native_locator_kind TEXT;
         ALTER TABLE nodes ADD COLUMN native_locator_encoding TEXT;
         ALTER TABLE nodes ADD COLUMN native_locator_raw BLOB;
         ALTER TABLE nodes ADD COLUMN self_modified_unix_seconds INTEGER;
         ALTER TABLE scan_staging ADD COLUMN native_locator_kind TEXT;
         ALTER TABLE scan_staging ADD COLUMN native_locator_encoding TEXT;
         ALTER TABLE scan_staging ADD COLUMN native_locator_raw BLOB;
         ALTER TABLE scan_staging ADD COLUMN self_modified_unix_seconds INTEGER;
         ALTER TABLE graph_revisions ADD COLUMN locator_writer_generation INTEGER NOT NULL DEFAULT 0;
         CREATE TRIGGER revisions_require_locator_writer BEFORE INSERT ON graph_revisions
             WHEN NEW.locator_writer_generation!=11
             BEGIN SELECT RAISE(ABORT,'obsolete locator writer; reopen with current DiskGraph'); END;
         PRAGMA user_version=11;",
    )?;
    // 历史显示路径和目录聚合时间不能反推原始字节、编码或自身修改时间。
    tx.commit()?;
    Ok(())
}
