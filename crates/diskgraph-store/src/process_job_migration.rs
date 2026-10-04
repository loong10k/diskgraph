//! 独立 process v1 输入与安全诊断的控制库 v9 迁移；来源：原生 Rust D42 / EC-02。
use crate::{ControlStore, Result, StoreError};
use rusqlite::{Connection, TransactionBehavior};
impl ControlStore {
    /// 参数：原控制连接；返回：独立表/固定输入门禁与版本同事务提交，错表整体回滚。
    pub(crate) fn migrate_process_job_inputs(connection: &Connection) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS process_evidence_job_inputs(
            job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL CHECK(schema_version=1),
            input_json TEXT NOT NULL CHECK(length(CAST(input_json AS BLOB))<=16384),
            input_sha256 TEXT NOT NULL CHECK(length(input_sha256)=64));
            CREATE TABLE IF NOT EXISTS process_job_failures(job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
                phase TEXT NOT NULL CHECK(length(CAST(phase AS BLOB))<=32 AND phase IN ('admission','execution','publication','reconciliation')),
                code TEXT NOT NULL CHECK(length(CAST(code AS BLOB))<=64 AND code IN ('budget_exceeded','timeout','cancelled','permission_denied','conflict','unsupported','unavailable','internal_error')));")?;
        Self::validate_process_job_input_schema(&tx)?;
        tx.execute_batch("CREATE TRIGGER IF NOT EXISTS process_job_input_immutable_update BEFORE UPDATE ON process_evidence_job_inputs BEGIN SELECT RAISE(ABORT,'Process job input is immutable'); END;
            CREATE TRIGGER IF NOT EXISTS process_job_input_immutable_delete BEFORE DELETE ON process_evidence_job_inputs WHEN EXISTS(SELECT 1 FROM jobs WHERE job_id=OLD.job_id) BEGIN SELECT RAISE(ABORT,'Process job input is immutable'); END;
            CREATE TRIGGER IF NOT EXISTS process_job_failure_no_update BEFORE UPDATE ON process_job_failures BEGIN SELECT RAISE(ABORT,'Process failure diagnostic is immutable'); END;
            CREATE TRIGGER IF NOT EXISTS process_job_failure_no_delete BEFORE DELETE ON process_job_failures WHEN EXISTS(SELECT 1 FROM jobs WHERE job_id=OLD.job_id) BEGIN SELECT RAISE(ABORT,'Process failure diagnostic is immutable'); END;
            CREATE TRIGGER IF NOT EXISTS process_input_requires_kind BEFORE INSERT ON process_evidence_job_inputs WHEN NOT EXISTS(SELECT 1 FROM jobs j WHERE j.job_id=NEW.job_id AND j.kind='process_evidence') OR EXISTS(SELECT 1 FROM git_evidence_job_inputs i WHERE i.job_id=NEW.job_id) BEGIN SELECT RAISE(ABORT,'Process input kind conflict'); END;
            CREATE TRIGGER IF NOT EXISTS git_input_rejects_process BEFORE INSERT ON git_evidence_job_inputs WHEN EXISTS(SELECT 1 FROM process_evidence_job_inputs i WHERE i.job_id=NEW.job_id) BEGIN SELECT RAISE(ABORT,'Git input kind conflict'); END;")?;
        tx.pragma_update(None, "user_version", 9)?;
        tx.commit()?;
        Ok(())
    }
    /// 参数：当前连接；返回：实际精确 v1 四列及固定诊断 schema，错表不启用服务。
    pub(crate) fn validate_process_job_input_schema(connection: &Connection) -> Result<()> {
        let columns: Vec<(String, String, i64, i64)> = connection
            .prepare("PRAGMA table_info(process_evidence_job_inputs)")?
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))?
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
                "invalid process job input schema".into(),
            ));
        }
        crate::process_job_failure_store::validate_schema(connection)
    }
}
