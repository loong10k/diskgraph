//! 请求能力与实时 scope 策略的交集；未发布策略保留可信内部兼容语义。
use crate::{ControlStore, Result, StoreError};
use diskgraph_core::{Permission, PrincipalId, ScopeId};
use rusqlite::{OptionalExtension, params};

impl ControlStore {
    /// 读取当前持久策略下指定主体的精确 scope 权限。
    /// 参数：principal 为真实主体，permission 为能力，scope 为资源实际归属。
    /// 返回：未发布策略为 None，已发布为实时允许/拒绝；缺失范围及损坏策略保留错误。
    pub fn policy_scope_permission(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<Option<bool>> {
        // 一次语句共享同一 WAL 观察，避免探测策略后再次观察 scope 与 grant。
        // 仅复用编译产物；不缓存允许结果，不保留跨请求读取事务。
        let state = self
            .connection
            .prepare_cached(
                "SELECT p.version,p.revoked,s.scope_id IS NOT NULL,s.revoked,
                EXISTS(SELECT 1 FROM grants g WHERE g.policy_version=p.version
                    AND p.revoked=0 AND g.principal_id=?1 AND g.permission=?2 AND g.scope_id=?3)
             FROM policy p LEFT JOIN scopes s ON s.scope_id=?3 WHERE p.id=1",
            )?
            .query_row(
                params![principal.as_str(), permission.wire_name(), scope.as_str()],
                |row| {
                    // 旧组合先解码策略的两列；scope 缺失或撤销不能掩盖损坏策略。
                    let _version = row.get::<_, i64>(0)?;
                    let _revoked = row.get::<_, i64>(1)?;
                    if !row.get::<_, bool>(2)? {
                        return Ok((false, false));
                    }
                    let allowed = row.get::<_, i64>(3)? == 0 && row.get::<_, bool>(4)?;
                    Ok((true, allowed))
                },
            )
            .optional()?;
        match state {
            None => Ok(None),
            Some((false, _)) => Err(StoreError::ScopeNotFound(scope.as_str().into())),
            Some((true, allowed)) => Ok(Some(allowed)),
        }
    }
}
