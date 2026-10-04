//! 会话预算之前的纯借用否认门禁；来源：Rust D42，不能替代随后严格 typed 解码或授予执行权。
use crate::{Result, StoreError};
use diskgraph_core::Permission;
use rusqlite::{Connection, params};

/// 参数：同一真实fence事务与job；返回：授权仍允许时的借用raw成本，失权优先于会话取消/预算。
/// SQLite JSON 运算只处理既有16KiB硬门禁内记录，不拥有 Rust authority/Input，也不做native I/O。
pub(crate) fn precheck(connection: &Connection, job_id: &str) -> Result<usize> {
    let mut statement = connection.prepare(
        "SELECT j.scope_id,j.principal,a.schema_version,
        CASE WHEN length(CAST(a.authority_json AS BLOB))<=16384 THEN a.authority_json END
        FROM jobs j LEFT JOIN job_request_authorities a ON a.job_id=j.job_id
        WHERE j.job_id=?1 AND j.kind='process_evidence'",
    )?;
    let mut rows = statement.query([job_id])?;
    let row = rows
        .next()?
        .ok_or_else(|| StoreError::InvalidGraph("budgeted fence requires Process job".into()))?;
    let fields = [
        row.get_ref(0)?,
        row.get_ref(1)?,
        row.get_ref(2)?,
        row.get_ref(3)?,
    ];
    let invalid = || StoreError::InvalidGraph("invalid persisted job authority".into());
    let scope = fields[0].as_str().map_err(|_| invalid())?;
    let principal = fields[1].as_str().map_err(|_| invalid())?;
    let raw = fields[3].as_str().map_err(|_| invalid())?;
    if row.get::<_, Option<i64>>(2)? != Some(1) {
        return Err(invalid());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StoreError::Conflict("job authority clock unavailable".into()))?
        .as_secs();
    let allowed: Option<bool> = connection.query_row(
        "SELECT CASE WHEN json_valid(?1) THEN json_extract(?1,'$.principal')=?2 AND
          ((json_extract(?1,'$.origin')='trusted_local' AND json_type(?1,'$.expires_at_unix_seconds')='null') OR
           (json_extract(?1,'$.origin')='authenticated_remote' AND json_type(?1,'$.expires_at_unix_seconds')='integer' AND json_extract(?1,'$.expires_at_unix_seconds')>?3)) END",
        params![raw,principal,now as i64], |r|r.get(0),
    )?;
    if !allowed.ok_or_else(invalid)? {
        return Err(StoreError::Conflict("job request authority denied".into()));
    }
    let remote: bool = connection.query_row(
        "SELECT json_extract(?1,'$.origin')='authenticated_remote'",
        [raw],
        |r| r.get(0),
    )?;
    let policy: bool =
        connection.query_row("SELECT EXISTS(SELECT 1 FROM policy WHERE id=1)", [], |r| {
            r.get(0)
        })?;
    if remote && !policy {
        return Err(StoreError::Conflict(
            "remote job requires live policy".into(),
        ));
    }
    // 此否认投影只需两种标量权限：持久 serde 标签与 grants 的 wire_name 是不同契约。
    // 不构造临时 Value/String；完整 typed authority 仍在准入后再次校验，不能由此投影授予权限。
    for (permission, serialized) in [
        (Permission::MetadataRead, "metadata_read"),
        (Permission::IndexWrite, "index_write"),
    ] {
        if remote {
            let ceiling: bool = connection.query_row(
                "SELECT json_type(?1,'$.capability_ceiling')='array' AND EXISTS(SELECT 1 FROM json_each(?1,'$.capability_ceiling') WHERE type='text' AND value=?2)",
                params![raw,serialized], |r|r.get(0),
            )?;
            if !ceiling {
                return Err(StoreError::Conflict("job request authority denied".into()));
            }
        }
        if policy {
            let granted: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM policy p JOIN grants g ON g.policy_version=p.version WHERE p.id=1 AND p.revoked=0 AND g.principal_id=?1 AND g.scope_id=?2 AND g.permission=?3)",
                params![principal,scope,permission.wire_name()], |r|r.get(0),
            )?;
            if !granted {
                return Err(StoreError::Conflict(
                    "live job authorization withdrawn".into(),
                ));
            }
        }
    }
    crate::metadata_read_cost::raw(&fields)
}
