//! 持久请求授权入队、严格 legacy 回收与 v7 一致性迁移。

use crate::{ControlStore, JobKind, JobRecord, Result, StoreError};
use diskgraph_core::{JobAuthorityOrigin, JobRequestAuthority, Permission, ScopeId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

impl ControlStore {
    /// 读取并验证原任务身份。参数：job_id 为真实持久任务。
    /// 返回：已验证上下文；None 明确表示旧来源未知，不能推断为可信本地。
    pub fn job_request_authority(&self, job_id: &str) -> Result<Option<JobRequestAuthority>> {
        crate::job_authority_gate::read(&self.connection, job_id)
    }

    /// 按不可变请求上下文入队，在同事务求能力与实时 grant 交集。
    /// 参数：实际 scope、扫描类型、服务端身份、主体活动配额；无 bearer 或客户端身份字段。
    /// 返回：新建/合并记录、配额拒绝 None 或授权/数据库错误。
    pub fn create_job_with_authority(
        &mut self,
        scope: &ScopeId,
        kind: JobKind,
        authority: &JobRequestAuthority,
        maximum: u64,
    ) -> Result<Option<JobRecord>> {
        if kind == JobKind::GitEvidence {
            return Err(StoreError::Conflict(
                "Git job requires typed fixed input".into(),
            ));
        }
        let encoded = serde_json::to_string(authority)?;
        if encoded.len() > 16384 {
            return Err(StoreError::InvalidGraph("oversized job authority".into()));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::job_authority_gate::validate_scope(
            &tx,
            scope,
            authority,
            &[Permission::IndexWrite],
        )?;
        // 仅本地旧扫描保留 Index/Sync 同范围合并；远程必须同 kind 及原 authority。
        let trusted_scan_merge = authority.origin() == JobAuthorityOrigin::TrustedLocal;
        let existing: Option<String> = tx.query_row(
            "SELECT j.job_id FROM jobs j JOIN job_request_authorities a ON a.job_id=j.job_id WHERE j.scope_id=?1 AND j.principal=?2 AND (j.kind=?3 OR (?4 AND j.kind IN ('index','sync'))) AND a.schema_version=1 AND a.authority_json=?5 AND j.state IN ('queued','running') ORDER BY j.created_at_unix_ms DESC,j.job_id DESC LIMIT 1",
            params![scope.as_str(),authority.principal().as_str(),kind.as_str(),trusted_scan_merge,encoded], |row| row.get(0),
        ).optional()?;
        if let Some(job_id) = existing {
            crate::job_authority_gate::validate_scope(
                &tx,
                scope,
                authority,
                &[Permission::IndexWrite],
            )?;
            tx.commit()?;
            return self.job(&job_id).map(Some);
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE principal=?1 AND state IN ('queued','running')",
            [authority.principal().as_str()],
            |row| row.get(0),
        )?;
        if count.max(0) as u64 >= maximum {
            return Ok(None);
        }
        let job_id = format!("job-{}", uuid::Uuid::new_v4());
        let now = Self::now_ms();
        tx.execute("INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,owner,principal) VALUES(?1,?2,?3,'queued',?4,?4,'',?5)", params![job_id,scope.as_str(),kind.as_str(),now as i64,authority.principal().as_str()])?;
        tx.execute("INSERT INTO job_request_authorities(job_id,schema_version,authority_json) VALUES(?1,1,?2)", params![job_id,encoded])?;
        // 真正控制库提交前再次检查，不以最初能力检查替代入队终态。
        crate::job_authority_gate::validate_scope(
            &tx,
            scope,
            authority,
            &[Permission::IndexWrite],
        )?;
        tx.commit()?;
        self.job(&job_id).map(Some)
    }

    /// 严格 runner 认领。参数：任务 ID 与本代 owner。
    /// 返回：唯一 fence；旧无上下文任务失败关闭，不抢占存活 running 租约。
    pub fn claim_job_once_strict(&mut self, job_id: &str, owner: &str) -> Result<JobRecord> {
        self.claim_job_authorized(job_id, owner, false, true)
    }

    /// 严格调度清理可认领但失去请求权限的任务。参数：无。
    /// 返回：本轮最多 64 个候选中的终结数量；只处理 queued/过期 running。
    /// 真实取消为 cancelled，其他拒绝为 failed；runner 直接逐项 strict claim，无需此额外扫描。
    pub fn reap_request_jobs_strict(&mut self) -> Result<u64> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Self::now_ms();
        let ids: Vec<String> = {
            let mut statement = tx.prepare("SELECT job_id FROM jobs WHERE state IN ('queued','running') AND (state='queued' OR (state='running' AND lease_expires_unix_ms<=?1)) ORDER BY created_at_unix_ms,job_id LIMIT 64")?;
            statement
                .query_map([now as i64], |row| row.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let mut changed = 0;
        for job_id in ids {
            let invalid = crate::job_authority_gate::validate_job(
                &tx,
                &job_id,
                &[Permission::IndexWrite],
                true,
            );
            match invalid {
                Err(StoreError::Conflict(_)) => {
                    changed += tx.execute("UPDATE jobs SET state=CASE WHEN cancel_requested=1 THEN 'cancelled' ELSE 'failed' END,heartbeat_unix_ms=?2 WHERE job_id=?1 AND (state='queued' OR (state='running' AND lease_expires_unix_ms<=?2))", params![job_id,now as i64])? as u64;
                }
                Err(error) => return Err(error),
                Ok(()) => {}
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// 将控制库一致性升级到 v7。参数：原控制连接；返回：完成提交或全部回滚。
    pub(crate) fn migrate_job_request_authorities(connection: &Connection) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS job_request_authorities(
            job_id TEXT PRIMARY KEY REFERENCES jobs(job_id) ON DELETE CASCADE,
            schema_version INTEGER NOT NULL CHECK(schema_version=1),
            authority_json TEXT NOT NULL CHECK(length(CAST(authority_json AS BLOB))<=16384));
            CREATE INDEX IF NOT EXISTS jobs_active_queue_order ON jobs(created_at_unix_ms,job_id) WHERE state IN ('queued','running');
            CREATE TRIGGER IF NOT EXISTS job_authority_immutable_update BEFORE UPDATE ON job_request_authorities BEGIN SELECT RAISE(ABORT,'job authority is immutable'); END;
            CREATE TRIGGER IF NOT EXISTS job_authority_immutable_delete BEFORE DELETE ON job_request_authorities WHEN EXISTS(SELECT 1 FROM jobs WHERE job_id=OLD.job_id) BEGIN SELECT RAISE(ABORT,'job authority is immutable'); END;
            ")?;
        Self::validate_job_authority_schema(&tx)?;
        tx.pragma_update(None, "user_version", 7)?;
        tx.commit()?;
        Ok(())
    }

    /// 验证实际 side table。参数：当前事务/连接；返回：精确列契约符合或明确错误。
    pub(crate) fn validate_job_authority_schema(connection: &Connection) -> Result<()> {
        // 允许旧迁移夹具保留已有的正确 side table；同名损坏表不得被 IF NOT EXISTS 掩盖。
        let columns: Vec<(String, String, i64, i64)> = connection
            .prepare("PRAGMA table_info(job_request_authorities)")?
            .query_map([], |row| {
                Ok((row.get(1)?, row.get(2)?, row.get(3)?, row.get(5)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let expected = vec![
            ("job_id".into(), "TEXT".into(), 0, 1),
            ("schema_version".into(), "INTEGER".into(), 1, 0),
            ("authority_json".into(), "TEXT".into(), 1, 0),
        ];
        if columns != expected {
            return Err(StoreError::InvalidGraph(
                "invalid job authority schema".into(),
            ));
        }
        Ok(())
    }
}
