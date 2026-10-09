//! 长连接仅判断真实主体是否仍有 token 所允许的实时授权。
use crate::{ControlStore, Result};
use diskgraph_core::{Permission, PrincipalId, ScopeId};
use rusqlite::params;

impl ControlStore {
    /// 判断长连接是否仍有权限。参数：真实主体、token 允许权限和固定管理范围。
    /// 返回：至少一项当前 epoch 授权有效；不缓存结果，不授予未发布策略兼容权限。
    pub fn identity_has_live_permission(
        &self,
        principal: &PrincipalId,
        permissions: &[Permission],
        admin_scope: &ScopeId,
    ) -> Result<bool> {
        if permissions.is_empty() {
            return Ok(false);
        }
        let Some((version, false)) = self.policy_state()? else {
            return Ok(false);
        };
        if version == 0 {
            return Ok(false);
        }
        // 主键前缀只查当前主体/权限；scope 按其主键点查，不物化全库策略和范围。
        // 每次执行重查策略 epoch，避免将换代前的版本与换代后的 grant 拼接。
        let mut statement = self.connection.prepare_cached(
            "SELECT g.scope_id,s.revoked FROM grants g
             JOIN policy p ON p.id=1 AND p.revoked=0 AND p.version=?3
                 AND g.policy_version=p.version
             LEFT JOIN scopes s ON s.scope_id=g.scope_id
             WHERE g.principal_id=?1 AND g.permission=?2",
        )?;
        for permission in permissions {
            let mut rows = statement.query(params![
                principal.as_str(),
                permission.wire_name(),
                version as i64
            ])?;
            while let Some(row) = rows.next()? {
                let scope: String = row.get(0)?;
                let Ok(scope) = ScopeId::new(scope) else {
                    continue;
                };
                // 管理范围沿用既有无注册记录语义；普通范围必须存在且未撤销。
                if &scope == admin_scope || row.get::<_, Option<i64>>(1)? == Some(0) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "live_identity_permission_tests.rs"]
mod tests;
