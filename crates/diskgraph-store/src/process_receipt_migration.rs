//! Process 回执及 Unix 强身份 side table 的图库 v14 一致性迁移；来源：原生 Rust D42 / EV-05。
use crate::{Result, StoreError};
use rusqlite::{Connection, TransactionBehavior};
/// 参数：原图库连接；返回：独立协议、互斥回执与版本同事务提交；Git v1 数据不重编码。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS process_job_publication_receipts(
        job_id TEXT PRIMARY KEY,schema_version INTEGER NOT NULL CHECK(schema_version=1),
        input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64),server_id TEXT NOT NULL,scope_id TEXT NOT NULL,
        snapshot_id TEXT NOT NULL,revision_id TEXT NOT NULL UNIQUE,run_id TEXT NOT NULL UNIQUE,node_id INTEGER NOT NULL CHECK(node_id>0),
        receipt_json TEXT NOT NULL CHECK(length(CAST(receipt_json AS BLOB))<=16384),writer_generation INTEGER NOT NULL CHECK(writer_generation=14));
        CREATE TABLE IF NOT EXISTS node_unix_observations(snapshot_id TEXT NOT NULL,node_id INTEGER NOT NULL,
            observation_raw BLOB,gap TEXT,writer_generation INTEGER NOT NULL CHECK(writer_generation=14),
            PRIMARY KEY(snapshot_id,node_id),FOREIGN KEY(snapshot_id,node_id) REFERENCES nodes(snapshot_id,id) ON DELETE CASCADE,
            CHECK((observation_raw IS NOT NULL AND gap IS NULL AND length(observation_raw)<=1024) OR (observation_raw IS NULL AND gap IS NOT NULL AND gap IN ('not_captured','unsupported','denied','changed','capture_failed','tree_mismatch'))));
        CREATE TABLE IF NOT EXISTS scan_staging_unix_observations(job_id TEXT NOT NULL,node_id INTEGER NOT NULL,
            observation_raw BLOB,gap TEXT,writer_generation INTEGER NOT NULL CHECK(writer_generation=14),PRIMARY KEY(job_id,node_id),
            CHECK((observation_raw IS NOT NULL AND gap IS NULL AND length(observation_raw)<=1024) OR (observation_raw IS NULL AND gap IS NOT NULL AND gap IN ('not_captured','unsupported','denied','changed','capture_failed','tree_mismatch'))));")?;
    ensure_staging_point_index(&tx)?;
    validate(&tx)?;
    tx.execute_batch("CREATE INDEX IF NOT EXISTS process_receipts_by_snapshot_target ON process_job_publication_receipts(snapshot_id,node_id,run_id);
        CREATE TRIGGER IF NOT EXISTS process_receipt_no_update BEFORE UPDATE ON process_job_publication_receipts BEGIN SELECT RAISE(ABORT,'Process receipt is immutable'); END;
        CREATE TRIGGER IF NOT EXISTS process_receipt_no_delete BEFORE DELETE ON process_job_publication_receipts BEGIN SELECT RAISE(ABORT,'Process receipt is immutable'); END;
        CREATE TRIGGER IF NOT EXISTS process_receipt_no_git_job BEFORE INSERT ON process_job_publication_receipts WHEN EXISTS(SELECT 1 FROM job_publication_receipts WHERE job_id=NEW.job_id OR revision_id=NEW.revision_id OR run_id=NEW.run_id) BEGIN SELECT RAISE(ABORT,'Process receipt conflicts with Git'); END;
        CREATE TRIGGER IF NOT EXISTS git_receipt_no_process_job BEFORE INSERT ON job_publication_receipts WHEN EXISTS(SELECT 1 FROM process_job_publication_receipts WHERE job_id=NEW.job_id OR revision_id=NEW.revision_id OR run_id=NEW.run_id) BEGIN SELECT RAISE(ABORT,'Git receipt conflicts with Process'); END;")?;
    tx.pragma_update(None, "user_version", 14)?;
    tx.commit()?;
    Ok(())
}
/// 为既有暂存数据建立节点 ID 点查索引；来源：原生 Rust D42 / FS-02。
/// 参数：当前连接或迁移事务；返回：索引成功，损坏 JSON/SQL 失败保持原事务回滚。
/// 非唯一索引保留可信旧暂存入口的重复值；观测准入仍用 COUNT(*)=1 拒绝歧义。
/// 建立索引须一次读取既有行，后续点查成本不再随整个任务暂存行数增长。
pub(crate) fn ensure_staging_point_index(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS scan_staging_by_job_node_id
         ON scan_staging(job_id,json_extract(node_json,'$.id'));",
    )?;
    Ok(())
}
/// 参数：当前连接；返回：receipt 和强身份旁表实际列匹配，损坏/旧缺协议拒绝。
pub(crate) fn validate(connection: &Connection) -> Result<()> {
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
    ];
    let columns: Vec<(String, String, i64, i64)> = connection
        .prepare("PRAGMA table_info(process_job_publication_receipts)")?
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))?
        .collect::<std::result::Result<_, _>>()?;
    if columns != expected.map(|(n, t, nn, pk)| (n.to_owned(), t.to_owned(), nn, pk)) {
        return Err(StoreError::InvalidGraph(
            "invalid process receipt schema".into(),
        ));
    }
    for (table, key) in [
        ("node_unix_observations", "snapshot_id"),
        ("scan_staging_unix_observations", "job_id"),
    ] {
        let cols: Vec<(String, String, i64, i64)> = connection
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let expected = [
            (key, "TEXT", 1, 1),
            ("node_id", "INTEGER", 1, 2),
            ("observation_raw", "BLOB", 0, 0),
            ("gap", "TEXT", 0, 0),
            ("writer_generation", "INTEGER", 1, 0),
        ];
        if cols != expected.map(|(n, t, nn, pk)| (n.to_owned(), t.to_owned(), nn, pk)) {
            return Err(StoreError::InvalidGraph(
                "invalid Unix observation side schema".into(),
            ));
        }
    }
    Ok(())
}
