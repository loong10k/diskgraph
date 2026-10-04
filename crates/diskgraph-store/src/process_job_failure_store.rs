//! 有 fence 的任务终态与固定诊断同事务持久化；来源：原生 Rust EC-02 / RT-03。
use crate::{ControlStore, JobRecord, JobState, Result, StoreError};
use diskgraph_core::{ProcessEvidenceFailure, ProcessEvidenceFailureCode};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

impl ControlStore {
    /// 参数：真实任务、原 owner/fence、终态与可选固定诊断；返回：条件结算或失效 owner。
    /// Failed/Cancelled 必须有诊断，Completed 只允许 None 且可信 Engine 已确认图库回执。
    /// 结算不复验过期认证，也不能产生新采集或发布；旧扫描接口/记录形状保持不变。
    pub fn finish_process_job_fenced(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        state: JobState,
        failure: Option<&ProcessEvidenceFailure>,
    ) -> Result<JobRecord> {
        match (state, failure) {
            (JobState::Completed, None) => {}
            (JobState::Failed, Some(d)) if d.code() != ProcessEvidenceFailureCode::Cancelled => {}
            (JobState::Cancelled, Some(d)) if d.code() == ProcessEvidenceFailureCode::Cancelled => {
            }
            _ => {
                return Err(StoreError::Conflict(
                    "invalid Process terminal diagnostic".into(),
                ));
            }
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Self::now_ms();
        let changed = tx.execute(
            "UPDATE jobs SET state=?4,heartbeat_unix_ms=?5 WHERE job_id=?1 AND kind='process_evidence'
            AND owner=?2 AND fencing_token=?3 AND state='running' AND lease_expires_unix_ms>?5",
            params![
                job_id,
                owner,
                crate::node_codec::as_i64(fence)?,
                state.as_str(),
                now as i64
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::StaleOwner);
        }
        if let Some(diagnostic) = failure {
            record(&tx, job_id, diagnostic)?;
        }
        tx.commit()?;
        self.job(job_id)
    }
    /// 参数：持久任务；返回：原固定诊断或 None，过长/未知标签不作为安全诊断输出。
    pub fn process_job_failure(&self, job_id: &str) -> Result<Option<ProcessEvidenceFailure>> {
        let process: Option<bool> = self
            .connection
            .query_row(
                "SELECT kind='process_evidence' FROM jobs WHERE job_id=?1",
                [job_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(process) = process else {
            return Err(StoreError::JobNotFound(job_id.into()));
        };
        if !process {
            return Ok(None);
        }
        let row:Option<(Option<String>,Option<String>)>=self.connection.query_row("SELECT
            CASE WHEN length(CAST(phase AS BLOB))<=32 THEN phase ELSE NULL END,
            CASE WHEN length(CAST(code AS BLOB))<=64 THEN code ELSE NULL END FROM process_job_failures WHERE job_id=?1",
            [job_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        match row {
            None => Ok(None),
            Some((Some(phase), Some(code))) => {
                serde_json::from_value(serde_json::json!({"phase":phase,"code":code}))
                    .map(Some)
                    .map_err(|_| {
                        StoreError::InvalidGraph("invalid Process failure diagnostic".into())
                    })
            }
            _ => Err(StoreError::InvalidGraph(
                "oversized Process failure diagnostic".into(),
            )),
        }
    }
}
/// 参数：同一控制事务、真实任务与固定诊断；返回：同事务保存或拒绝，不覆盖原终态依据。
pub(crate) fn record(
    connection: &Connection,
    job_id: &str,
    diagnostic: &ProcessEvidenceFailure,
) -> Result<()> {
    connection.execute(
        "INSERT INTO process_job_failures VALUES(?1,?2,?3)",
        params![
            job_id,
            diagnostic.phase().as_str(),
            diagnostic.code().as_str()
        ],
    )?;
    Ok(())
}

/// 参数：当前控制事务/连接；返回：实际三列精确匹配，否则拒绝启用。
pub(crate) fn validate_schema(connection: &Connection) -> Result<()> {
    let columns: Vec<(String, String, i64, i64)> = connection
        .prepare("PRAGMA table_info(process_job_failures)")?
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))?
        .collect::<std::result::Result<_, _>>()?;
    if columns
        != [
            ("job_id".into(), "TEXT".into(), 0, 1),
            ("phase".into(), "TEXT".into(), 1, 0),
            ("code".into(), "TEXT".into(), 1, 0),
        ]
    {
        return Err(StoreError::InvalidGraph(
            "invalid Process failure diagnostic schema".into(),
        ));
    }
    Ok(())
}

/// 参数：同控制事务、固定筛选列和实际键；返回：仅真实 queued Process 的安全取消诊断。
/// SQL 列只能取内部白名单；状态 UPDATE 由同事务调用方执行，Running 不提前写终态。
pub(crate) fn record_queued_cancel(connection: &Connection, column: &str, key: &str) -> Result<()> {
    if !matches!(column, "job_id" | "scope_id") {
        return Err(StoreError::InvalidGraph(
            "invalid internal cancellation selector".into(),
        ));
    }
    connection.execute(&format!("INSERT INTO process_job_failures(job_id,phase,code) SELECT job_id,'admission','cancelled' FROM jobs WHERE {column}=?1 AND kind='process_evidence' AND state='queued'"),[key])?;
    Ok(())
}
