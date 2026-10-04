//! 同一控制连接的固定 Process 目标验证；来源：原生 Rust EC-02 / SC-06。
use crate::{JobKind, Result, StoreError};
use diskgraph_core::ProcessEvidenceJobInput;
use rusqlite::Connection;

/// 参数：连接、实际任务；返回：严格固定输入或缺失/损坏错误，不猜测客户端路径。
pub(crate) fn read(connection: &Connection, job_id: &str) -> Result<ProcessEvidenceJobInput> {
    read_with_admission(connection, job_id, &mut |_, _, _| Ok(()))
}

/// 参数：同一连接、任务与原会话准入；返回：事前 raw/分配累计准入后严格解码的固定输入。
pub(crate) fn read_with_admission(
    connection: &Connection,
    job_id: &str,
    admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
) -> Result<ProcessEvidenceJobInput> {
    admit(0, 0, 0)?;
    with_raw(connection, job_id, |raw, scope, server, digest, bytes| {
        crate::metadata_read_cost::charge(admit, bytes, raw.len(), 4096, 1)?;
        let input: ProcessEvidenceJobInput = serde_json::from_str(raw).map_err(|_| invalid())?;
        if input.scope_id().as_str() != scope
            || input.server_id().as_str() != server
            || input.digest() != digest
        {
            return Err(invalid());
        }
        admit(0, 0, 0)?;
        Ok(input)
    })
}

/// 参数：连接、真实任务与借用消费者；返回：硬准入后的投影，未拥有完整输入。
pub(crate) fn with_raw<T>(
    connection: &Connection,
    job_id: &str,
    consume: impl FnOnce(&str, &str, &str, &str, usize) -> Result<T>,
) -> Result<T> {
    let mut statement = connection.prepare(
        "SELECT j.scope_id,j.kind,i.schema_version,i.input_sha256,
        CASE WHEN length(CAST(i.input_json AS BLOB))<=16384 THEN i.input_json ELSE NULL END,
        (SELECT server_id FROM server WHERE id=1)
        FROM jobs j LEFT JOIN process_evidence_job_inputs i ON i.job_id=j.job_id WHERE j.job_id=?1",
    )?;
    let mut rows = statement.query([job_id])?;
    let row = rows
        .next()?
        .ok_or_else(|| StoreError::JobNotFound(job_id.into()))?;
    // JSON 和冗余身份列共用原始准入；任何大字段都不得先拥有，再用等值比较拒绝。
    let fields = [0, 1, 3, 4, 5].map(|column| row.get_ref(column)?.as_str().map_err(|_| invalid()));
    let [scope, kind, digest, raw, server] = fields;
    let (scope, kind, digest, raw, server) = (scope?, kind?, digest?, raw?, server?);
    let bytes = [scope, kind, digest, raw, server]
        .iter()
        .try_fold(0_usize, |total, field| total.checked_add(field.len()))
        .ok_or_else(invalid)?;
    if bytes > 16384
        || kind != JobKind::ProcessEvidence.as_str()
        || row.get::<_, Option<i64>>(2)? != Some(1)
    {
        return Err(invalid());
    }
    // 原有字符串硬门禁不变；实际读取还包含 schema_version 这个 INTEGER。
    let read_bytes = bytes.checked_add(8).ok_or(StoreError::BudgetExceeded)?;
    consume(raw, scope, server, digest, read_bytes)
}

fn invalid() -> StoreError {
    StoreError::InvalidGraph("invalid or missing Process job input".into())
}

/// 参数：连接、候选输入；返回：当前服务器与真实范围匹配，否则明确拒绝。
pub(crate) fn validate_target(
    connection: &Connection,
    input: &ProcessEvidenceJobInput,
) -> Result<()> {
    let matches: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM server WHERE id=1 AND server_id=?1)",
        [input.server_id().as_str()],
        |row| row.get(0),
    )?;
    if !matches {
        return Err(StoreError::Conflict("foreign Process server".into()));
    }
    Ok(())
}
