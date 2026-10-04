//! 同一控制事务中的持久请求上限及实时权限交集，不持有另一个授权 owner。

use crate::{JobKind, Result, StoreError};
use diskgraph_core::{JobAuthorityOrigin, JobRequestAuthority, Permission, ScopeId};
use rusqlite::{Connection, OptionalExtension, params};

/// 读取真实持久身份。参数：现有控制连接和任务 ID；返回：已验证身份或旧来源未知 None。
pub(crate) fn read(connection: &Connection, job_id: &str) -> Result<Option<JobRequestAuthority>> {
    read_with_admission(connection, job_id, &mut |_, _, _| Ok(()))
}

/// 参数：同一连接、真实job及原会话准入；返回：raw/拥有分配事前准入后的严格身份。
pub(crate) fn read_with_admission(
    connection: &Connection,
    job_id: &str,
    admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
) -> Result<Option<JobRequestAuthority>> {
    admit(0, 0, 0)?;
    // 先由 SQLite 长度准入，再拥有小型 JSON；缺行明确为 LegacyUnknown。
    let mut statement = connection.prepare(
        "SELECT j.principal,a.schema_version,CASE WHEN length(CAST(a.authority_json AS BLOB))<=16384 THEN a.authority_json ELSE NULL END FROM jobs j LEFT JOIN job_request_authorities a ON a.job_id=j.job_id WHERE j.job_id=?1",
    )?;
    let mut rows = statement.query([job_id])?;
    let row = rows
        .next()?
        .ok_or_else(|| StoreError::JobNotFound(job_id.into()))?;
    let fields = [row.get_ref(0)?, row.get_ref(1)?, row.get_ref(2)?];
    let invalid = || StoreError::InvalidGraph("invalid persisted job authority".into());
    let principal = fields[0].as_str().map_err(|_| invalid())?;
    let json = match fields[2] {
        rusqlite::types::ValueRef::Null => None,
        value => Some(value.as_str().map_err(|_| invalid())?),
    };
    crate::metadata_read_cost::charge(
        admit,
        crate::metadata_read_cost::raw(&fields)?,
        json.map_or(0, str::len),
        4096,
        1,
    )?;
    let authority = match (row.get::<_, Option<i64>>(1)?, json) {
        (None, None) => None,
        (Some(1), Some(json)) => {
            let authority: JobRequestAuthority = serde_json::from_str(json)
                .map_err(|_| StoreError::InvalidGraph("invalid persisted job authority".into()))?;
            if authority.principal().as_str() != principal {
                return Err(StoreError::InvalidGraph(
                    "job authority principal mismatch".into(),
                ));
            }
            // Deserialize 本身验证来源不变量并规范化能力；不把远程缺项降级本地。
            Some(authority)
        }
        _ => {
            return Err(StoreError::InvalidGraph(
                "unsupported or oversized job authority".into(),
            ));
        }
    };
    admit(0, 0, 0)?;
    Ok(authority)
}

/// 求能力上限与当前授权交集。参数：连接、实际 scope、原身份、所需权限；返回：准入或拒绝。
pub(crate) fn validate_scope(
    connection: &Connection,
    scope: &ScopeId,
    authority: &JobRequestAuthority,
    required: &[Permission],
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StoreError::Conflict("job authority clock unavailable".into()))?
        .as_secs();
    if required
        .iter()
        .any(|permission| !authority.allows(permission, now))
    {
        return Err(StoreError::Conflict("job request authority denied".into()));
    }
    let revoked: bool = connection
        .query_row(
            "SELECT revoked FROM scopes WHERE scope_id=?1",
            [scope.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| StoreError::ScopeNotFound(scope.as_str().into()))?;
    if revoked {
        return Err(StoreError::Conflict("job scope revoked".into()));
    }
    let policy_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM policy WHERE id=1)",
        [],
        |row| row.get(0),
    )?;
    if !policy_exists {
        return if authority.origin() == JobAuthorityOrigin::TrustedLocal {
            Ok(())
        } else {
            Err(StoreError::Conflict(
                "remote job requires live policy".into(),
            ))
        };
    }
    for permission in required {
        let allowed: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM policy p JOIN grants g ON g.policy_version=p.version WHERE p.id=1 AND p.revoked=0 AND g.principal_id=?1 AND g.scope_id=?2 AND g.permission=?3)",
            params![authority.principal().as_str(),scope.as_str(),permission.wire_name()], |row| row.get(0),
        )?;
        if !allowed {
            return Err(StoreError::Conflict(
                "live job authorization withdrawn".into(),
            ));
        }
    }
    // SQL 与锁等待会消耗认证时间；最后再检查原 exp，绝不续期。
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StoreError::Conflict("job authority clock unavailable".into()))?
        .as_secs();
    authority
        .validate_at(now)
        .map_err(|_| StoreError::Conflict("job request authority expired".into()))
}

/// 复核原任务身份。参数：连接、任务、服务定义的权限和 strict；返回：准入或真实格式/授权错误。
pub(crate) fn validate_job(
    connection: &Connection,
    job_id: &str,
    required: &[Permission],
    strict: bool,
) -> Result<()> {
    validate_job_with_admission(connection, job_id, required, strict, &mut |_, _, _| Ok(()))
}

/// 参数：同一事务/真实job/所需权限/strict与原会话；返回：借用先准入后的原授权判断。
pub(crate) fn validate_job_with_admission(
    connection: &Connection,
    job_id: &str,
    required: &[Permission],
    strict: bool,
    admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
) -> Result<()> {
    admit(0, 0, 0)?;
    let mut statement = connection.prepare("SELECT kind,scope_id FROM jobs WHERE job_id=?1")?;
    let mut rows = statement.query([job_id])?;
    let row = rows
        .next()?
        .ok_or_else(|| StoreError::JobNotFound(job_id.into()))?;
    let fields = [row.get_ref(0)?, row.get_ref(1)?];
    crate::metadata_read_cost::charge(
        admit,
        crate::metadata_read_cost::raw(&fields)?,
        0,
        std::mem::size_of::<ScopeId>(),
        1,
    )?;
    let kind = JobKind::from_str(
        fields[0]
            .as_str()
            .map_err(|_| StoreError::InvalidGraph("invalid job kind".into()))?,
    )
    .ok_or_else(|| StoreError::InvalidGraph("invalid job kind".into()))?;
    let scope = ScopeId::new(
        fields[1]
            .as_str()
            .map_err(|_| StoreError::InvalidGraph("invalid job scope".into()))?,
    )
    .map_err(|_| StoreError::InvalidGraph("invalid job scope".into()))?;
    let cancelled: bool = connection.query_row(
        "SELECT cancel_requested FROM jobs WHERE job_id=?1",
        [job_id],
        |row| row.get(0),
    )?;
    if cancelled {
        return Err(StoreError::Conflict("job cancellation requested".into()));
    }
    if kind == JobKind::GitEvidence {
        // 新产品 callback 仅用于 Process；旧 Git 固定协议与可信入口保持既有路径。
        crate::git_job_input_codec::read(connection, job_id)?;
    }
    if kind == JobKind::ProcessEvidence {
        crate::process_job_input_codec::read_with_admission(connection, job_id, admit)?;
    }
    match read_with_admission(connection, job_id, admit)? {
        Some(authority) => {
            // 固定 kind 权限先复验；旧调用方传空/窄集合也不能降 Git ContentRead。
            validate_scope(connection, &scope, &authority, kind.required_permissions())?;
            let extra_allocation = required
                .len()
                .checked_mul(std::mem::size_of::<Permission>())
                .and_then(|n| u64::try_from(n).ok())
                .ok_or(StoreError::BudgetExceeded)?;
            admit(0, 0, extra_allocation)?;
            let extra: Vec<_> = required
                .iter()
                .copied()
                .filter(|p| !kind.required_permissions().contains(p))
                .collect();
            if extra.is_empty() {
                Ok(())
            } else {
                validate_scope(connection, &scope, &authority, &extra)
            }
        }
        None if strict || matches!(kind, JobKind::GitEvidence | JobKind::ProcessEvidence) => Err(
            StoreError::Conflict("legacy job request authority unknown".into()),
        ),
        None => Ok(()),
    }
}
