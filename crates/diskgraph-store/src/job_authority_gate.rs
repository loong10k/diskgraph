//! 同一控制事务中的持久请求上限及实时权限交集，不持有另一个授权 owner。

use crate::{Result, StoreError};
use diskgraph_core::{JobAuthorityOrigin, JobRequestAuthority, Permission, ScopeId};
use rusqlite::{Connection, OptionalExtension, params};

/// 读取真实持久身份。参数：现有控制连接和任务 ID；返回：已验证身份或旧来源未知 None。
pub(crate) fn read(connection: &Connection, job_id: &str) -> Result<Option<JobRequestAuthority>> {
    // 先由 SQLite 长度准入，再拥有小型 JSON；缺行明确为 LegacyUnknown。
    let row: Option<(String, Option<i64>, Option<String>)> = connection.query_row(
        "SELECT j.principal,a.schema_version,CASE WHEN length(CAST(a.authority_json AS BLOB))<=16384 THEN a.authority_json ELSE NULL END FROM jobs j LEFT JOIN job_request_authorities a ON a.job_id=j.job_id WHERE j.job_id=?1",
        [job_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?;
    let (principal, version, json) = row.ok_or_else(|| StoreError::JobNotFound(job_id.into()))?;
    match (version, json) {
        (None, None) => Ok(None),
        (Some(1), Some(json)) => {
            let authority: JobRequestAuthority = serde_json::from_str(&json)
                .map_err(|_| StoreError::InvalidGraph("invalid persisted job authority".into()))?;
            if authority.principal().as_str() != principal {
                return Err(StoreError::InvalidGraph(
                    "job authority principal mismatch".into(),
                ));
            }
            // Deserialize 本身验证来源不变量并规范化能力；不把远程缺项降级本地。
            Ok(Some(authority))
        }
        _ => Err(StoreError::InvalidGraph(
            "unsupported or oversized job authority".into(),
        )),
    }
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
    match read(connection, job_id)? {
        Some(authority) => {
            let scope: String = connection.query_row(
                "SELECT scope_id FROM jobs WHERE job_id=?1",
                [job_id],
                |row| row.get(0),
            )?;
            let scope = ScopeId::new(scope)
                .map_err(|_| StoreError::InvalidGraph("invalid job scope".into()))?;
            validate_scope(connection, &scope, &authority, required)
        }
        None if strict => Err(StoreError::Conflict(
            "legacy job request authority unknown".into(),
        )),
        None => Ok(()),
    }
}
