//! 同一控制连接的固定 Git 目标验证；来源：原生 Rust EC-02 / SC-06。
use crate::{JobKind, Result, StoreError};
use diskgraph_core::GitEvidenceJobInput;
use rusqlite::{Connection, OptionalExtension};

/// 参数：连接、实际任务；返回：严格固定输入或缺失/损坏错误，不猜测客户端路径。
pub(crate) fn read(connection: &Connection, job_id: &str) -> Result<GitEvidenceJobInput> {
    let mut statement = connection.prepare(
        "SELECT j.scope_id,j.kind,i.schema_version,i.input_sha256,
        CASE WHEN length(CAST(i.input_json AS BLOB))<=16384 THEN i.input_json ELSE NULL END,
        (SELECT server_id FROM server WHERE id=1)
        FROM jobs j LEFT JOIN git_evidence_job_inputs i ON i.job_id=j.job_id WHERE j.job_id=?1",
    )?;
    let mut rows = statement.query([job_id])?;
    let row = rows
        .next()?
        .ok_or_else(|| StoreError::JobNotFound(job_id.into()))?;
    let invalid = || StoreError::InvalidGraph("invalid or missing Git job input".into());
    // JSON 和冗余身份列共用原始准入；任何大字段都不得先拥有，再用等值比较拒绝。
    let fields = [0, 1, 3, 4, 5].map(|column| row.get_ref(column)?.as_str().map_err(|_| invalid()));
    let [scope, kind, digest, raw, server] = fields;
    let (scope, kind, digest, raw, server) = (scope?, kind?, digest?, raw?, server?);
    let bytes = [scope, kind, digest, raw, server]
        .iter()
        .try_fold(0_usize, |total, field| total.checked_add(field.len()))
        .ok_or_else(invalid)?;
    if bytes > 16384
        || kind != JobKind::GitEvidence.as_str()
        || row.get::<_, Option<i64>>(2)? != Some(1)
    {
        return Err(invalid());
    }
    let input: GitEvidenceJobInput = serde_json::from_str(raw).map_err(|_| invalid())?;
    if input.scope_id().as_str() != scope
        || input.server_id().as_str() != server
        || input.digest() != digest
    {
        return Err(invalid());
    }
    Ok(input)
}

/// 参数：连接、候选输入；返回：当前服务器与真实范围匹配，否则明确拒绝。
pub(crate) fn validate_target(connection: &Connection, input: &GitEvidenceJobInput) -> Result<()> {
    let server: Option<String> = connection
        .query_row("SELECT server_id FROM server WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()?;
    if server.as_deref() != Some(input.server_id().as_str()) {
        return Err(StoreError::Conflict("foreign Git server".into()));
    }
    Ok(())
}
