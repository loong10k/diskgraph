//! 有界固定 Process 任务入队；来源：原生 Rust D44 / EC-04，旧可信 create 不变。
use crate::control_write_deadline::ControlWriteDeadline;
use crate::{ControlStore, JobKind, JobRecord, Result, StoreError};
use diskgraph_core::{JobRequestAuthority, ProcessEvidenceJobInput};
use rusqlite::{OptionalExtension, params};
use std::time::Instant;

/// 参数：独占现有控制库、固定输入、原身份/配额/期限；返回：同事务确认的任务或真实拒绝。
pub(crate) fn create(
    store: &ControlStore,
    input: &ProcessEvidenceJobInput,
    authority: &JobRequestAuthority,
    maximum: u64,
    deadline: Instant,
) -> Result<Option<JobRecord>> {
    let window = ControlWriteDeadline::new(
        &store.connection,
        deadline,
        authority.expires_at_unix_seconds(),
    )?;
    let result = (|| {
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
        let digest = input.digest();
        window.begin()?;
        let connection = &store.connection;
        window.statement(|| crate::process_job_input_codec::validate_target(connection, input))?;
        window.statement(|| {
            crate::job_authority_gate::validate_scope(
                connection,
                input.scope_id(),
                authority,
                JobKind::ProcessEvidence.required_permissions(),
            )
        })?;
        let existing: Option<String> = window.statement(|| Ok(connection.query_row(
            "SELECT j.job_id FROM jobs j JOIN job_request_authorities a ON a.job_id=j.job_id
             JOIN process_evidence_job_inputs i ON i.job_id=j.job_id WHERE j.kind='process_evidence' AND j.scope_id=?1 AND j.principal=?2
             AND j.state IN ('queued','running') AND a.schema_version=1 AND a.authority_json=?3 AND i.schema_version=1
             AND i.input_sha256=?4 AND i.input_json=?5 ORDER BY j.created_at_unix_ms DESC,j.job_id DESC LIMIT 1",
            params![input.scope_id().as_str(), authority.principal().as_str(), auth, digest, encoded],
            |row| row.get(0),
        ).optional()?))?;
        if let Some(id) = existing {
            window.statement(|| crate::process_job_input_codec::read(connection, &id))?;
            let job = window.statement(|| store.job(&id))?;
            window.statement(|| {
                crate::job_authority_gate::validate_scope(
                    connection,
                    input.scope_id(),
                    authority,
                    JobKind::ProcessEvidence.required_permissions(),
                )
            })?;
            window.commit()?;
            return Ok(Some(job));
        }
        let count: i64 = window.statement(|| {
            Ok(connection.query_row(
                "SELECT COUNT(*) FROM jobs WHERE principal=?1 AND state IN ('queued','running')",
                [authority.principal().as_str()],
                |row| row.get(0),
            )?)
        })?;
        if count.max(0) as u64 >= maximum {
            window.check()?;
            return Ok(None);
        }
        let id = format!("job-{}", uuid::Uuid::new_v4());
        let now = ControlStore::now_ms();
        window.statement(|| Ok(connection.execute(
            "INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,owner,principal) VALUES(?1,?2,'process_evidence','queued',?3,?3,'',?4)",
            params![id, input.scope_id().as_str(), now as i64, authority.principal().as_str()],
        )?))?;
        window.statement(|| {
            Ok(connection.execute(
                "INSERT INTO job_request_authorities VALUES(?1,1,?2)",
                params![id, auth],
            )?)
        })?;
        window.statement(|| {
            Ok(connection.execute(
                "INSERT INTO process_evidence_job_inputs VALUES(?1,1,?2,?3)",
                params![id, encoded, digest],
            )?)
        })?;
        // JobRecord 在真正提交之前取得；之后不再新读控制库或重建结果期限。
        let job = window.statement(|| store.job(&id))?;
        window.statement(|| {
            crate::job_authority_gate::validate_scope(
                connection,
                input.scope_id(),
                authority,
                JobKind::ProcessEvidence.required_permissions(),
            )
        })?;
        window.commit()?;
        Ok(Some(job))
    })();
    window.finish(result)
}
