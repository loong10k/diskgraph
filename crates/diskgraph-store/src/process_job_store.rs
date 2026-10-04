//! 固定目标、原请求身份与任务同事务入队；来源：原生 Rust EC-02 / SC-06。
use crate::{ControlStore, JobKind, JobRecord, Result, StoreError};
use diskgraph_core::{JobRequestAuthority, ProcessEvidenceJobInput};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl ControlStore {
    /// 参数：服务端确认的固定目标、原身份和活动配额；返回：同输入/同身份合并、新任务或配额 None。
    /// 本可信存储入口不读取资源；revision/node 的图库授权仍由唯一 Engine 完成。
    pub fn create_process_evidence_job(
        &mut self,
        input: &ProcessEvidenceJobInput,
        authority: &JobRequestAuthority,
        maximum: u64,
    ) -> Result<Option<JobRecord>> {
        let encoded = serde_json::to_string(input)?;
        let auth = serde_json::to_string(authority)?;
        if encoded
            .len()
            .saturating_add(input.scope_id().as_str().len())
            .saturating_add(input.server_id().as_str().len())
            .saturating_add(80)
            > 16384
            || auth.len() > 16384
        {
            return Err(StoreError::InvalidGraph(
                "oversized Process job input".into(),
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::process_job_input_codec::validate_target(&tx, input)?;
        crate::job_authority_gate::validate_scope(
            &tx,
            input.scope_id(),
            authority,
            JobKind::ProcessEvidence.required_permissions(),
        )?;
        let existing:Option<String>=tx.query_row("SELECT j.job_id FROM jobs j JOIN job_request_authorities a ON a.job_id=j.job_id
            JOIN process_evidence_job_inputs i ON i.job_id=j.job_id WHERE j.kind='process_evidence' AND j.scope_id=?1 AND j.principal=?2
            AND j.state IN ('queued','running') AND a.schema_version=1 AND a.authority_json=?3 AND i.schema_version=1
            AND i.input_sha256=?4 AND i.input_json=?5 ORDER BY j.created_at_unix_ms DESC,j.job_id DESC LIMIT 1",
            params![input.scope_id().as_str(),authority.principal().as_str(),auth,input.digest(),encoded],|row|row.get(0)).optional()?;
        if let Some(id) = existing {
            crate::process_job_input_codec::read(&tx, &id)?;
            crate::job_authority_gate::validate_scope(
                &tx,
                input.scope_id(),
                authority,
                JobKind::ProcessEvidence.required_permissions(),
            )?;
            tx.commit()?;
            return self.job(&id).map(Some);
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE principal=?1 AND state IN ('queued','running')",
            [authority.principal().as_str()],
            |row| row.get(0),
        )?;
        if count.max(0) as u64 >= maximum {
            return Ok(None);
        }
        let id = format!("job-{}", uuid::Uuid::new_v4());
        let now = Self::now_ms();
        tx.execute("INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,owner,principal) VALUES(?1,?2,'process_evidence','queued',?3,?3,'',?4)",
            params![id,input.scope_id().as_str(),now as i64,authority.principal().as_str()])?;
        tx.execute(
            "INSERT INTO job_request_authorities VALUES(?1,1,?2)",
            params![id, auth],
        )?;
        tx.execute(
            "INSERT INTO process_evidence_job_inputs VALUES(?1,1,?2,?3)",
            params![id, encoded, input.digest()],
        )?;
        crate::job_authority_gate::validate_scope(
            &tx,
            input.scope_id(),
            authority,
            JobKind::ProcessEvidence.required_permissions(),
        )?;
        tx.commit()?;
        self.job(&id).map(Some)
    }

    /// 参数：真实 Process 任务 ID；返回：重新验证的原固定输入，缺失或损坏明确失败。
    pub fn process_evidence_job_input(&self, job_id: &str) -> Result<ProcessEvidenceJobInput> {
        crate::process_job_input_codec::read(&self.connection, job_id)
    }
}
