//! 为后台调度器读取有限数量的真实可认领记录，旧全量入口继续兼容。

use crate::{ControlStore, JobRecord, Result, StoreError};
use rusqlite::params;

impl ControlStore {
    /// 读取有界的队列窗口。参数：maximum 为本轮最大任务数，0 返回空集合。
    /// 返回：按创建时间和任务 ID 稳定排序的可认领任务；不会拥有窗口外的任务字段。
    /// 来源：DiskGraph 原生 Rust SC-06 请求任务调度；不改变可信旧全量 API。
    pub fn list_queued_jobs_limited(&self, maximum: usize) -> Result<Vec<JobRecord>> {
        let limit = i64::try_from(maximum).map_err(|_| StoreError::IntegerOverflow)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(
            "SELECT jobs.job_id FROM jobs JOIN scopes ON scopes.scope_id=jobs.scope_id
             WHERE jobs.state IN ('queued','running') AND ((scopes.revoked=0 AND jobs.cancel_requested=0)
                 OR EXISTS(SELECT 1 FROM git_evidence_job_inputs i WHERE i.job_id=jobs.job_id)
                 OR jobs.kind IN ('index','sync','process_evidence'))
               AND (jobs.state='queued' OR (jobs.state='running' AND jobs.lease_expires_unix_ms<=?1))
             ORDER BY jobs.created_at_unix_ms ASC,jobs.job_id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![Self::now_ms() as i64, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(self.job(&row?)?);
        }
        Ok(jobs)
    }
}
