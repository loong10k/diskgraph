//! v11 -> v12 完整 Windows 原生观测与独立发布代次。

use crate::Result;
use rusqlite::Connection;

/// 原子增加观测字段及写入门禁，历史数据保持全空。
/// 参数：connection 为待升级的图库连接。
/// 返回：迁移提交成功；数据库失败时全部回滚。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let tx = connection.unchecked_transaction()?;
    tx.execute_batch(
        "ALTER TABLE nodes ADD COLUMN native_observation_format TEXT;
         ALTER TABLE nodes ADD COLUMN native_observation_raw BLOB;
         ALTER TABLE nodes ADD COLUMN native_observation_gap TEXT;
         ALTER TABLE scan_staging ADD COLUMN native_observation_format TEXT;
         ALTER TABLE scan_staging ADD COLUMN native_observation_raw BLOB;
         ALTER TABLE scan_staging ADD COLUMN native_observation_gap TEXT;
         ALTER TABLE graph_revisions ADD COLUMN native_observation_writer_generation INTEGER NOT NULL DEFAULT 0;
         CREATE TRIGGER revisions_require_native_observation_writer BEFORE INSERT ON graph_revisions
             WHEN NEW.native_observation_writer_generation!=12
             BEGIN SELECT RAISE(ABORT,'obsolete native observation writer; reopen with current DiskGraph'); END;
         PRAGMA user_version=12;",
    )?;
    tx.commit()?;
    Ok(())
}
