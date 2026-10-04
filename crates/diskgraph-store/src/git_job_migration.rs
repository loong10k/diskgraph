//! 固定 Git 输入的 v8 一致性迁移；来源：原生 Rust EC-02 / RT-01。
use crate::{ControlStore, Result, StoreError};
use rusqlite::{Connection, TransactionBehavior};

impl ControlStore {
    /// 参数：已有控制连接；返回：新表与版本同事务提交或整体回滚。
    pub(crate) fn migrate_git_job_inputs(connection: &Connection) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS git_evidence_job_inputs(
            job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL CHECK(schema_version=1),
            input_json TEXT NOT NULL CHECK(length(CAST(input_json AS BLOB))<=16384),
            input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64));
            CREATE TRIGGER IF NOT EXISTS git_job_input_immutable_update BEFORE UPDATE ON git_evidence_job_inputs BEGIN SELECT RAISE(ABORT,'Git job input is immutable'); END;
            CREATE TRIGGER IF NOT EXISTS git_job_input_immutable_delete BEFORE DELETE ON git_evidence_job_inputs WHEN EXISTS(SELECT 1 FROM jobs WHERE job_id=OLD.job_id) BEGIN SELECT RAISE(ABORT,'Git job input is immutable'); END;
            CREATE TABLE IF NOT EXISTS git_job_failures(job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
                phase TEXT NOT NULL CHECK(length(CAST(phase AS BLOB))<=32 AND phase IN ('admission','execution','publication','reconciliation')),
                code TEXT NOT NULL CHECK(length(CAST(code AS BLOB))<=64 AND code IN ('budget_exceeded','timeout','cancelled','permission_denied','conflict','unsupported','unavailable','internal_error')));
            CREATE TRIGGER IF NOT EXISTS git_job_failure_no_update BEFORE UPDATE ON git_job_failures BEGIN SELECT RAISE(ABORT,'Git failure diagnostic is immutable'); END;
            CREATE TRIGGER IF NOT EXISTS git_job_failure_no_delete BEFORE DELETE ON git_job_failures WHEN EXISTS(SELECT 1 FROM jobs WHERE job_id=OLD.job_id) BEGIN SELECT RAISE(ABORT,'Git failure diagnostic is immutable'); END;")?;
        Self::validate_git_job_input_schema(&tx)?;
        tx.pragma_update(None, "user_version", 8)?;
        tx.commit()?;
        Ok(())
    }
    /// 参数：当前控制连接；返回：实际列精确匹配或损坏错误，IF NOT EXISTS 不掩盖旧错表。
    pub(crate) fn validate_git_job_input_schema(connection: &Connection) -> Result<()> {
        let columns: Vec<(String, String, i64, i64)> = connection
            .prepare("PRAGMA table_info(git_evidence_job_inputs)")?
            .query_map([], |row| {
                Ok((row.get(1)?, row.get(2)?, row.get(3)?, row.get(5)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        if columns
            != [
                ("job_id".into(), "TEXT".into(), 0, 1),
                ("schema_version".into(), "INTEGER".into(), 1, 0),
                ("input_json".into(), "TEXT".into(), 1, 0),
                ("input_sha256".into(), "TEXT".into(), 1, 0),
            ]
        {
            return Err(StoreError::InvalidGraph(
                "invalid Git job input schema".into(),
            ));
        }
        crate::git_job_failure_store::validate_schema(connection)?;
        Ok(())
    }
}
