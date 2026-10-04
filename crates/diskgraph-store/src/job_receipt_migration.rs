//! 独立 Git 回执协议，不改变原文件/locator/Windows 观察代次；来源：原生 Rust RT-01。
use crate::{Result, StoreError};
use rusqlite::{Connection, TransactionBehavior};

/// 参数：已有图库连接；返回：回执与 schema13 同事务提交，错表导致全部回滚。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS job_publication_receipts(
        job_id TEXT PRIMARY KEY,
        schema_version INTEGER NOT NULL CHECK(schema_version=1),
        input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64),
        server_id TEXT NOT NULL,scope_id TEXT NOT NULL,snapshot_id TEXT NOT NULL,
        revision_id TEXT NOT NULL UNIQUE,run_id TEXT NOT NULL UNIQUE,node_id INTEGER NOT NULL CHECK(node_id>0),
        receipt_json TEXT NOT NULL CHECK(length(CAST(receipt_json AS BLOB))<=16384),
        writer_generation INTEGER NOT NULL CHECK(writer_generation=13));
        CREATE INDEX IF NOT EXISTS receipts_by_snapshot_target ON job_publication_receipts(snapshot_id,node_id,run_id);
        CREATE TRIGGER IF NOT EXISTS job_receipt_no_update BEFORE UPDATE ON job_publication_receipts BEGIN SELECT RAISE(ABORT,'publication receipt is immutable'); END;
        CREATE TRIGGER IF NOT EXISTS job_receipt_no_delete BEFORE DELETE ON job_publication_receipts BEGIN SELECT RAISE(ABORT,'publication receipt is immutable'); END;")?;
    validate(&tx)?;
    tx.pragma_update(None, "user_version", 13)?;
    tx.commit()?;
    Ok(())
}
/// 参数：当前图库连接；返回：实际精确 schema 或拒绝，旧缺协议 writer 不能插入新回执。
pub(crate) fn validate(connection: &Connection) -> Result<()> {
    let columns: Vec<(String, String, i64, i64)> = connection
        .prepare("PRAGMA table_info(job_publication_receipts)")?
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let expected = [
        ("job_id", "TEXT", 0, 1),
        ("schema_version", "INTEGER", 1, 0),
        ("input_sha256", "TEXT", 1, 0),
        ("server_id", "TEXT", 1, 0),
        ("scope_id", "TEXT", 1, 0),
        ("snapshot_id", "TEXT", 1, 0),
        ("revision_id", "TEXT", 1, 0),
        ("run_id", "TEXT", 1, 0),
        ("node_id", "INTEGER", 1, 0),
        ("receipt_json", "TEXT", 1, 0),
        ("writer_generation", "INTEGER", 1, 0),
    ]
    .map(|(n, t, not_null, pk)| (n.to_owned(), t.to_owned(), not_null, pk));
    if columns != expected {
        return Err(StoreError::InvalidGraph(
            "invalid publication receipt schema".into(),
        ));
    }
    Ok(())
}
