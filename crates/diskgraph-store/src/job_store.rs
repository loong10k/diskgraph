//! 持久任务、条件认领、租约 fencing 和取消回收。

use rusqlite::OptionalExtension;

use crate::{ControlStore, JobKind, JobRecord, JobState, Result, StoreError};
use diskgraph_core::{PrincipalId, ScopeId};
use rusqlite::params;
use uuid::Uuid;

impl ControlStore {
    /// Creates a durable job for a live scope. An active job for the same
    /// scope is returned instead of duplicated (RT-01 job merging, AI-03).
    /// The creating principal is recorded so per-principal quotas can count it.
    /// 在持久配额与合并规则下创建或查询任务。
    /// 参数：scope_id：实际所属范围 ID；kind：节点或任务类型；principal：真实请求主体。
    /// 返回：合并/创建或更新后的持久任务，保留真实状态、owner 与 fence。
    pub fn create_job(
        &mut self,
        scope_id: &ScopeId,
        kind: JobKind,
        principal: &PrincipalId,
    ) -> Result<JobRecord> {
        self.create_job_internal(scope_id, kind, principal, u64::MAX, false)?
            .ok_or_else(|| StoreError::Conflict("job quota exceeded".into()))
    }

    /// 在单个写事务内按真实主体合并并检查配额，防止跨进程重复入队。
    /// 在持久配额与合并规则下创建或查询任务。
    /// 参数：scope_id：实际所属范围 ID；kind：任务类型；principal：真实请求主体；maximum：该主体的活动任务配额上限。
    /// 返回：Some 为新建或合并后的活动任务；None 表示活动配额已耗尽；撤权或数据库失败返回错误。
    pub fn create_job_with_quota(
        &mut self,
        scope_id: &ScopeId,
        kind: JobKind,
        principal: &PrincipalId,
        maximum: u64,
    ) -> Result<Option<JobRecord>> {
        self.create_job_internal(scope_id, kind, principal, maximum, true)
    }

    /// 在持久配额与合并规则下创建或查询任务。
    /// 参数：scope_id：实际所属范围 ID；kind：任务类型；principal：真实请求主体；maximum：该主体的活动任务配额上限；require_live_grant：是否强制持久实时 IndexWrite 授权。
    /// 返回：Some 为新建或合并后的活动任务；None 表示活动配额已耗尽；撤权或数据库失败返回错误。
    pub(crate) fn create_job_internal(
        &mut self,
        scope_id: &ScopeId,
        kind: JobKind,
        principal: &PrincipalId,
        maximum: u64,
        require_live_grant: bool,
    ) -> Result<Option<JobRecord>> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let revoked: bool = tx
            .query_row(
                "SELECT revoked FROM scopes WHERE scope_id = ?1",
                [scope_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::ScopeNotFound(scope_id.as_str().to_owned()))?;
        if revoked {
            return Err(StoreError::Conflict(format!("scope {scope_id} is revoked")));
        }
        if require_live_grant {
            let authorized: bool = tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM policy WHERE id = 1) OR EXISTS(SELECT 1 FROM policy p JOIN grants g ON g.policy_version = p.version WHERE p.id = 1 AND p.revoked = 0 AND g.principal_id = ?1 AND g.scope_id = ?2 AND g.permission = 'index:write')",params![principal.as_str(), scope_id.as_str()],|row| row.get(0))?;
            if !authorized {
                return Err(StoreError::Conflict(
                    "live index authorization was withdrawn".into(),
                ));
            }
        }
        let existing: Option<String>=tx.query_row("SELECT job_id FROM jobs WHERE scope_id = ?1 AND principal = ?2 AND state IN ('queued','running') AND NOT EXISTS(SELECT 1 FROM job_request_authorities a WHERE a.job_id=jobs.job_id) ORDER BY created_at_unix_ms DESC, job_id DESC LIMIT 1",params![scope_id.as_str(),principal.as_str()],|row| row.get(0)).optional()?;
        if let Some(job_id) = existing {
            tx.commit()?;
            return self.job(&job_id).map(Some);
        }
        let active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE principal = ?1 AND state IN ('queued','running')",
            [principal.as_str()],
            |row| row.get(0),
        )?;
        if active.max(0) as u64 >= maximum {
            return Ok(None);
        }
        let job_id = format!("job-{}", Uuid::new_v4());
        let now = Self::now_ms();
        tx.execute("INSERT INTO jobs (job_id, scope_id, kind, state, created_at_unix_ms, heartbeat_unix_ms, owner, principal) VALUES (?1,?2,?3,'queued',?4,?4,'',?5)",params![job_id,scope_id.as_str(),kind.as_str(),now as i64,principal.as_str()])?;
        tx.commit()?;
        self.job(&job_id).map(Some)
    }

    /// Every queued job, oldest first, across scopes. The job runner drains
    /// this list; nothing here is tied to a connection.
    /// 在持久配额与合并规则下创建或查询任务。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：`Result<Vec<JobRecord>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn list_queued_jobs(&self) -> Result<Vec<JobRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT jobs.job_id FROM jobs JOIN scopes ON scopes.scope_id = jobs.scope_id
             WHERE scopes.revoked = 0 AND jobs.cancel_requested = 0
               AND (jobs.state = 'queued' OR (jobs.state = 'running' AND jobs.lease_expires_unix_ms <= ?1))
             ORDER BY jobs.created_at_unix_ms ASC, jobs.job_id ASC",
        )?;
        let rows = statement.query_map([Self::now_ms() as i64], |row| row.get::<_, String>(0))?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(self.job(&row?)?);
        }
        Ok(jobs)
    }

    /// Ends expired jobs that revocation or cancellation made unclaimable.
    /// 条件终结过期且因取消/范围撤销不可认领的任务。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：成功条件终结的任务数量。
    pub fn reap_unclaimable_jobs(&mut self) -> Result<u64> {
        let changed = self.connection.execute(
            "UPDATE jobs SET state = 'cancelled', heartbeat_unix_ms = ?1
             WHERE state = 'running' AND lease_expires_unix_ms <= ?1
               AND (cancel_requested = 1 OR EXISTS
                   (SELECT 1 FROM scopes WHERE scopes.scope_id = jobs.scope_id AND scopes.revoked = 1))",
            [Self::now_ms() as i64],
        )?;
        Ok(changed as u64)
    }

    /// 仅终结指定 ID 的过期且因取消/范围撤销不可认领任务。
    /// 返回是否条件更新成功；存活租约、已完成和其他任务均保持不变。
    /// 条件终结过期且因取消/范围撤销不可认领的任务。
    /// 参数：job_id：持久任务 ID。
    /// 返回：是否成功终结且仅终结指定过期任务。
    pub fn reap_unclaimable_job(&mut self, job_id: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE jobs SET state = 'cancelled', heartbeat_unix_ms = ?2
             WHERE job_id = ?1 AND state = 'running' AND lease_expires_unix_ms <= ?2
               AND (cancel_requested = 1 OR EXISTS
                   (SELECT 1 FROM scopes WHERE scopes.scope_id = jobs.scope_id AND scopes.revoked = 1))",
            params![job_id, Self::now_ms() as i64],
        )?;
        Ok(changed == 1)
    }

    /// Active (queued or running) jobs attributed to one principal. Merged
    /// jobs count once, because the merge returns the existing record.
    /// 在持久配额与合并规则下创建或查询任务。
    /// 参数：principal：真实请求主体。
    /// 返回：确证计数；数据库或整数超界返回错误。
    pub fn active_job_count_for_principal(&self, principal: &PrincipalId) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM jobs
             WHERE principal = ?1 AND state IN ('queued', 'running')",
            [principal.as_str()],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }

    /// Loads one job record.
    /// 在持久配额与合并规则下创建或查询任务。
    /// 参数：job_id：持久任务 ID。
    /// 返回：合并/创建或更新后的持久任务，保留真实状态、owner 与 fence。
    pub fn job(&self, job_id: &str) -> Result<JobRecord> {
        self.connection
            .query_row(
                "SELECT scope_id, kind, state, created_at_unix_ms, heartbeat_unix_ms, owner, principal, fencing_token, lease_expires_unix_ms
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                |row| {
                    let principal = row.get::<_, String>(6).unwrap_or_default();
                    let principal = PrincipalId::new(if principal.is_empty() { "legacy-unbound".to_owned() } else { principal }).map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?;
                    Ok(JobRecord {
                        job_id: job_id.to_owned(),
                        scope_id: ScopeId::new(row.get::<_, String>(0)?).map_err(|error| {
                            rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                        })?,
                        kind: JobKind::from_str(&row.get::<_, String>(1)?).ok_or(
                            rusqlite::Error::InvalidColumnType(
                                1,
                                "kind".into(),
                                rusqlite::types::Type::Text,
                            ),
                        )?,
                        state: JobState::from_str(&row.get::<_, String>(2)?).ok_or(
                            rusqlite::Error::InvalidColumnType(
                                2,
                                "state".into(),
                                rusqlite::types::Type::Text,
                            ),
                        )?,
                        created_at_unix_ms: row.get::<_, i64>(3)?.try_into().unwrap_or(0),
                        heartbeat_unix_ms: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
                        owner: row.get(5)?,
                        fencing_token: row.get::<_, i64>(7)?.max(0) as u64,
                        lease_expires_unix_ms: row.get::<_, i64>(8)?.max(0) as u64,
                        principal,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::JobNotFound(job_id.to_owned()))
    }

    /// The one active (queued or running) job for a scope, newest first.
    /// 在持久配额与合并规则下创建或查询任务。
    /// 参数：scope_id：实际所属范围 ID。
    /// 返回：`Result<Option<JobRecord>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn active_job_for_scope(&self, scope_id: &ScopeId) -> Result<Option<JobRecord>> {
        let job_id: Option<String> = self
            .connection
            .query_row(
                "SELECT job_id FROM jobs
                 WHERE scope_id = ?1 AND state IN ('queued', 'running')
                 ORDER BY created_at_unix_ms DESC, job_id DESC LIMIT 1",
                [scope_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match job_id {
            Some(job_id) => Ok(Some(self.job(&job_id)?)),
            None => Ok(None),
        }
    }

    /// Transitions a queued job to running under `owner` (fencing token).
    /// A running job owned by someone else refuses the claim.
    /// 按条件 owner/lease/fencing 认领、续租或写入任务终态。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner。
    /// 返回：认领记录，保留可信同 owner 的幂等兼容。
    pub fn claim_job(&mut self, job_id: &str, owner: &str) -> Result<JobRecord> {
        self.claim_job_internal(job_id, owner, true)
    }

    /// runner 严格认领；同名 owner 也不能再次取得尚未过期的 running job。
    /// 参数为任务 ID 与本次执行 owner，返回唯一认领代次。
    /// 按条件 owner/lease/fencing 认领、续租或写入任务终态。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner。
    /// 返回：唯一认领的 owner/lease/fence 记录，竞争或失效返回错误。
    pub fn claim_job_once(&mut self, job_id: &str, owner: &str) -> Result<JobRecord> {
        self.claim_job_internal(job_id, owner, false)
    }

    /// 按条件 owner/lease/fencing 认领、续租或写入任务终态。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner；trusted_idempotent：是否允许可信同 owner 重复确认。
    /// 返回：合并/创建或更新后的持久任务，保留真实状态、owner 与 fence。
    pub(crate) fn claim_job_internal(
        &mut self,
        job_id: &str,
        owner: &str,
        trusted_idempotent: bool,
    ) -> Result<JobRecord> {
        self.claim_job_authorized(job_id, owner, trusted_idempotent, false)
    }

    /// 刷新当前 owner 的租约；已过期的 owner 不得续租。
    /// 按条件 owner/lease/fencing 认领、续租或写入任务终态。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn heartbeat(&mut self, job_id: &str, owner: &str) -> Result<()> {
        let job = self.job(job_id)?;
        self.heartbeat_fenced(job_id, owner, job.fencing_token)
    }

    /// 携带 fencing token 续租，防止同名 owner 的旧请求复活。
    /// 按条件 owner/lease/fencing 认领、续租或写入任务终态。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner；fence：条件认领所得 fencing token。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn heartbeat_fenced(&mut self, job_id: &str, owner: &str, fence: u64) -> Result<()> {
        self.heartbeat_job_authorized(job_id, owner, fence)
    }

    /// 在控制库写事务保护下验证租约并执行 staging/发布，跨进程认领不能插入中间。
    /// 在控制事务中复核 owner/fence、期限、取消及实时授权。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner；fence：条件认领所得 fencing token；work：有效事务期间的内部回调。
    /// 返回：fence 与授权全部满足时的回调结果；失效 owner、取消/撤权或过期拒绝。
    pub fn with_job_fence<T>(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        work: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.with_job_authority_fence(
            job_id,
            owner,
            fence,
            &[diskgraph_core::Permission::IndexWrite],
            work,
        )
    }

    /// 在原 fence 事务内检查持久请求上限及全部所需实时权限。
    /// 参数：job/owner/fence 指向当前代次，required 是服务定义的权限集合，work 为可信回调。
    /// 返回：全部准入后的回调结果；回调不得重入 control，图库需在自身 commit 前做纯时钟末检。
    /// 旧未绑定入口保留原可信语义；Some authority 即使经旧方法调用也不得绕过。
    pub fn with_job_authority_fence<T>(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        required: &[diskgraph_core::Permission],
        work: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let valid: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM jobs j JOIN scopes s ON s.scope_id = j.scope_id WHERE j.job_id = ?1 AND j.owner = ?2 AND j.fencing_token = ?3 AND j.state = 'running' AND j.cancel_requested = 0 AND j.lease_expires_unix_ms > ?4 AND s.revoked = 0 AND (NOT EXISTS (SELECT 1 FROM policy WHERE id = 1) OR EXISTS (SELECT 1 FROM policy p JOIN grants g ON g.policy_version = p.version WHERE p.id = 1 AND p.revoked = 0 AND g.principal_id = j.principal AND g.scope_id = j.scope_id AND g.permission = 'index:write')))", params![job_id, owner, fence as i64, Self::now_ms() as i64], |row| row.get(0))?;
        if !valid {
            return Err(StoreError::StaleOwner);
        }
        crate::job_authority_gate::validate_job(&tx, job_id, required, false)?;
        let result = work()?;
        // 回调可能已提交图库；不得声称此处再拒绝能回滚独立图库事务。
        tx.commit()?;
        Ok(result)
    }

    /// Persists cancellation across Engine instances. Terminal jobs are unchanged.
    /// 持久处理取消意图或排队终态，不承诺瞬时停止扫描。
    /// 参数：job_id：持久任务 ID。
    /// 返回：指定条件是否成立；数据库失败返回错误。
    pub fn request_cancel(&mut self, job_id: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE jobs SET cancel_requested = 1,
                 state = CASE WHEN state = 'queued' THEN 'cancelled' ELSE state END,
                 heartbeat_unix_ms = ?2
             WHERE job_id = ?1 AND state IN ('queued', 'running')",
            params![job_id, Self::now_ms() as i64],
        )?;
        if changed == 0 {
            self.job(job_id)?;
        }
        Ok(changed > 0)
    }

    /// Reads cancellation for the exact claim; stale generations are stopped.
    /// 持久处理取消意图或排队终态，不承诺瞬时停止扫描。
    /// 参数：job_id：持久任务 ID；fence：条件认领所得 fencing token。
    /// 返回：指定条件是否成立；数据库失败返回错误。
    pub fn cancellation_requested(&self, job_id: &str, fence: u64) -> Result<bool> {
        let cancelled: Option<bool> = self
            .connection
            .query_row(
                "SELECT cancel_requested != 0 OR state = 'cancelled' FROM jobs
             WHERE job_id = ?1 AND fencing_token = ?2",
                params![job_id, fence as i64],
                |row| row.get(0),
            )
            .optional()?;
        Ok(cancelled.unwrap_or(true))
    }

    /// Cancels a job that is still queued. Returns whether it was cancelled;
    /// running jobs are the runner's responsibility (RT-02).
    /// 持久处理取消意图或排队终态，不承诺瞬时停止扫描。
    /// 参数：job_id：持久任务 ID。
    /// 返回：指定条件是否成立；数据库失败返回错误。
    pub fn cancel_queued(&mut self, job_id: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE jobs SET state = 'cancelled', heartbeat_unix_ms = ?2
             WHERE job_id = ?1 AND state = 'queued'",
            params![job_id, Self::now_ms() as i64],
        )?;
        Ok(changed > 0)
    }

    /// Moves a running job to a terminal state; only the current owner may.
    /// 按条件 owner/lease/fencing 认领、续租或写入任务终态。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner；state：目标生命周期状态。
    /// 返回：合并/创建或更新后的持久任务，保留真实状态、owner 与 fence。
    pub fn finish_job(&mut self, job_id: &str, owner: &str, state: JobState) -> Result<JobRecord> {
        let job = self.job(job_id)?;
        self.finish_job_fenced(job_id, owner, job.fencing_token, state)
    }

    /// 仅当前租约代次可结束任务；同名 owner 的旧代次也被拒绝。
    /// 按条件 owner/lease/fencing 认领、续租或写入任务终态。
    /// 参数：job_id：持久任务 ID；owner：本代次执行 owner；fence：条件认领所得 fencing token；state：目标生命周期状态。
    /// 返回：合并/创建或更新后的持久任务，保留真实状态、owner 与 fence。
    pub fn finish_job_fenced(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        state: JobState,
    ) -> Result<JobRecord> {
        if !matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return Err(StoreError::Conflict(
                "finish_job requires a terminal state".into(),
            ));
        }
        let changed = self.connection.execute(
            "UPDATE jobs SET state = ?2, heartbeat_unix_ms = ?3 WHERE job_id = ?1 AND owner = ?4 AND fencing_token = ?5 AND state = 'running' AND lease_expires_unix_ms > ?3",
            params![job_id, state.as_str(), Self::now_ms() as i64, owner, fence as i64],
        )?;
        if changed != 1 {
            return Err(StoreError::StaleOwner);
        }
        self.job(job_id)
    }
}
