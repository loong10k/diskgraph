//! v16 保存扫描提交事实，不与可回收的历史建立级联关系；来源：原生 Rust RT-01。
use crate::{Result, StoreError};
use rusqlite::Connection;

const TABLE: &str = "CREATE TABLE scan_publication_receipts(job_id TEXT PRIMARY KEY CHECK(length(job_id) BETWEEN 1 AND 256),revision_id TEXT NOT NULL UNIQUE CHECK(length(revision_id) BETWEEN 1 AND 256),publishing_fence INTEGER NOT NULL CHECK(typeof(publishing_fence)='integer' AND publishing_fence>0),receipt_json TEXT NOT NULL CHECK(length(CAST(receipt_json AS BLOB)) BETWEEN 2 AND 16384))";
const NO_UPDATE: &str = "CREATE TRIGGER scan_receipt_no_update BEFORE UPDATE ON scan_publication_receipts BEGIN SELECT RAISE(ABORT,'scan publication receipt is immutable'); END";
const NO_DELETE: &str = "CREATE TRIGGER scan_receipt_no_delete BEFORE DELETE ON scan_publication_receipts BEGIN SELECT RAISE(ABORT,'scan publication receipt is immutable'); END";

const WRITER: &str = "CREATE TRIGGER snapshots_require_scan_receipt_writer BEFORE INSERT ON snapshots WHEN NEW.scan_receipt_writer != 16 BEGIN SELECT RAISE(ABORT,'scan publication writer is obsolete; reopen with current DiskGraph'); END";

/// 参数：已完成 v15 迁移的唯一图库连接；返回：账本和版本原子升级，失败不启用新 schema。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let tx = connection.unchecked_transaction()?;
    let columns: Vec<String> = tx
        .prepare("PRAGMA table_info(snapshots)")?
        .query_map([], |r| r.get(1))?
        .collect::<std::result::Result<_, _>>()?;
    if !columns.iter().any(|c| c == "scan_receipt_writer") {
        tx.execute_batch(
            "ALTER TABLE snapshots ADD COLUMN scan_receipt_writer INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    for sql in [TABLE, NO_UPDATE, NO_DELETE, WRITER] {
        let sql = if sql.starts_with("CREATE TABLE ") {
            sql.replacen("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ", 1)
        } else {
            sql.replacen("CREATE TRIGGER ", "CREATE TRIGGER IF NOT EXISTS ", 1)
        };
        tx.execute_batch(&sql)?;
    }
    validate(&tx)?;
    tx.pragma_update(None, "user_version", 16)?;
    tx.commit()?;
    Ok(())
}

/// 参数：原连接；返回：精确表与不可变触发器协议，缺失或替换一律拒绝启动。
pub(crate) fn validate(connection: &Connection) -> Result<()> {
    for (name, expected) in [
        ("scan_publication_receipts", TABLE),
        ("scan_receipt_no_update", NO_UPDATE),
        ("scan_receipt_no_delete", NO_DELETE),
        ("snapshots_require_scan_receipt_writer", WRITER),
    ] {
        let actual: String =
            connection.query_row("SELECT sql FROM sqlite_schema WHERE name=?1", [name], |r| {
                r.get(0)
            })?;
        if actual != expected {
            return Err(StoreError::InvalidGraph(
                "scan receipt schema differs from expected protocol".into(),
            ));
        }
    }
    let shape: (String,i64,String) = connection.query_row(
        "SELECT type,[notnull],dflt_value FROM pragma_table_info('snapshots') WHERE name='scan_receipt_writer'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    )?;
    if shape != ("INTEGER".into(), 1, "0".into()) {
        return Err(StoreError::InvalidGraph(
            "invalid scan writer generation column".into(),
        ));
    }
    Ok(())
}
