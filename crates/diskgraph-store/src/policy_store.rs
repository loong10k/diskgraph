//! 实时授权、策略 epoch、撤权与幂等 grant。

use rusqlite::OptionalExtension;

use crate::control_codec::parse_permission;
use crate::{ControlStore, Result, StoreError};
use diskgraph_core::{Grant, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use rusqlite::params;

impl ControlStore {
    /// Current policy version (0 until the first publish).
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：当前有效 epoch，未发布或已撤销为 0。
    pub fn policy_version(&self) -> Result<u64> {
        let row: Option<(i64, i64)> = self
            .connection
            .prepare_cached("SELECT version, revoked FROM policy WHERE id = 1")?
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        match row {
            Some((version, 0)) => Ok(version.max(0) as u64),
            _ => Ok(0),
        }
    }

    /// Publishes a new policy version; older grants stop applying.
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：version：新的策略版本 epoch。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn publish_policy_version(&mut self, version: u64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO policy (id, version, revoked) VALUES (1, ?1, 0)
             ON CONFLICT(id) DO UPDATE SET version = ?1, revoked = 0",
            [version as i64],
        )?;
        Ok(())
    }

    /// Revokes the whole policy; nothing is authorized until republished.
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn revoke_policy(&mut self) -> Result<()> {
        self.connection
            .execute("UPDATE policy SET revoked = 1 WHERE id = 1", [])?;
        Ok(())
    }

    /// Upserts one grant bound to the current policy version.
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：grant：完整主体/权限/范围/epoch 授权。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn upsert_grant(&mut self, grant: &Grant) -> Result<()> {
        self.upsert_grant_on_connection(grant)
    }

    /// 参数：固定 epoch 的 grant；返回：当前事务内幂等写入，不另建事务。
    pub(crate) fn upsert_grant_on_connection(&self, grant: &Grant) -> Result<()> {
        // 重复本地 bootstrap 只读已有键，避免每个查询进程重写权限和争夺提交锁。
        let existing: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM grants WHERE principal_id=?1 AND permission=?2 AND scope_id=?3 AND policy_version=?4)",
            params![grant.principal.as_str(), grant.permission.wire_name(), grant.scope.as_str(), grant.policy_version as i64],
            |row| row.get(0),
        )?;
        if existing {
            return Ok(());
        }
        self.connection.execute(
            "INSERT INTO grants (principal_id, permission, scope_id, policy_version)
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            params![
                grant.principal.as_str(),
                grant.permission.wire_name(),
                grant.scope.as_str(),
                grant.policy_version as i64,
            ],
        )?;
        Ok(())
    }

    /// Withdraws a grant across every policy version it was recorded under.
    ///
    /// The table's key includes the version, so a grant can exist more than
    /// once; removing only the current one would leave the right standing
    /// under an epoch that has not been reached yet.
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：principal：真实请求主体；permission：精确能力权限；scope：实际所属范围 ID。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn revoke_grant(
        &mut self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<()> {
        // 当前策略与真实 DELETE 结果属于同一个写事务；不在事务外猜测删除哪一代授权。
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let epoch: Option<i64> = tx
            .query_row(
                "SELECT p.version FROM policy p JOIN scopes s ON s.scope_id=?1
             WHERE p.id=1 AND p.revoked=0 AND s.revoked=0",
                [scope.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let mut removed_current = false;
        {
            let mut delete = tx.prepare(
                "DELETE FROM grants WHERE principal_id=?1 AND permission=?2 AND scope_id=?3
                 RETURNING policy_version",
            )?;
            let mut rows = delete.query(params![
                principal.as_str(),
                permission.wire_name(),
                scope.as_str()
            ])?;
            while let Some(row) = rows.next()? {
                let removed_epoch: i64 = row.get(0)?;
                removed_current |= epoch == Some(removed_epoch);
            }
        }
        // 即使宿主添加了重插 grant 的触发器，也不能把同一事务最终仍允许的状态报告为撤权。
        let notice = if removed_current {
            let still_allowed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM policy p JOIN grants g ON g.policy_version=p.version
                 JOIN scopes s ON s.scope_id=g.scope_id WHERE p.id=1 AND p.revoked=0 AND s.revoked=0
                 AND g.principal_id=?1 AND g.permission=?2 AND g.scope_id=?3)",
                params![principal.as_str(), permission.wire_name(), scope.as_str()],
                |row| row.get(0),
            )?;
            if still_allowed {
                None
            } else {
                epoch
                    .map(|epoch| {
                        crate::withdrawal_store::transaction_generation(&tx)
                            .map(|generation| (epoch.max(0) as u64, generation))
                    })
                    .transpose()?
            }
        } else {
            None
        };
        tx.commit()?;
        #[cfg(test)]
        crate::withdrawal_publish_hook::after_commit();
        if let Some((epoch, generation)) = notice {
            crate::withdrawal_registry::publish_grant(
                self, principal, scope, permission, epoch, generation,
            );
        }
        Ok(())
    }

    /// The raw policy row: None when never published, Some((version, revoked))
    /// after. Distinct from `policy_version`, which flattens revocation into 0
    /// and therefore cannot distinguish "never published" from "revoked".
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：None 表示从未发布策略；Some((version, revoked)) 保留版本与撤销两维。
    pub fn policy_state(&self) -> Result<Option<(u64, bool)>> {
        let row: Option<(i64, i64)> = self
            .connection
            .prepare_cached("SELECT version, revoked FROM policy WHERE id = 1")?
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        Ok(row.map(|(version, revoked)| (version.max(0) as u64, revoked != 0)))
    }

    /// 查询一个权限的实时授权；无持久策略时返回 None，供可信内部兼容调用使用。
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：principal：真实请求主体；permission：精确能力权限；scope：实际所属范围 ID。
    /// 返回：None 仅供未发布策略的可信兼容入口；Some(true/false) 为实时授权/拒绝，撤销范围返回 false。
    pub fn live_permission(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<Option<bool>> {
        // 一条 SQL 共享同一 WAL 观察，避免把撤权前的 scope 与撤权后的策略拼接。
        // 不开启跨请求事务、不缓存允许结果；按旧顺序解码必要字段，保留拒权及损坏语义。
        // 只复用有界连接缓存中的编译语句；重新绑定参数并执行，绝不缓存允许结果。
        self.connection
            .prepare_cached(
                "SELECT s.revoked, p.id, p.version, p.revoked,
                EXISTS(SELECT 1 FROM grants g WHERE g.policy_version=p.version
                    AND p.revoked=0 AND g.principal_id=?1 AND g.permission=?2 AND g.scope_id=?3)
             FROM scopes s LEFT JOIN policy p ON p.id=1 WHERE s.scope_id=?3",
            )?
            .query_row(
                params![principal.as_str(), permission.wire_name(), scope.as_str()],
                |row| {
                    if row.get::<_, i64>(0)? != 0 {
                        return Ok(Some(false));
                    }
                    if row.get::<_, Option<i64>>(1)?.is_none() {
                        return Ok(None);
                    }
                    // policy_state 原接口会解码两列，不能让 EXISTS 掩盖损坏策略。
                    let _version = row.get::<_, i64>(2)?;
                    let _revoked = row.get::<_, i64>(3)?;
                    Ok(Some(row.get(4)?))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::ScopeNotFound(scope.as_str().into()))
    }

    /// Builds the live authorizer from stored policy and grants.
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：按真实版本和撤销状态重建的授权器。
    pub fn authorizer(&self) -> Result<PolicyAuthorizer> {
        self.authorizer_filtered(None)
    }

    /// 仅重建指定真实主体的授权快照，不授予其他主体能力。
    /// 参数：principal 为已认证请求主体；返回：保留版本、撤销和默认拒绝的窄策略。
    pub fn authorizer_for_principal(&self, principal: &PrincipalId) -> Result<PolicyAuthorizer> {
        self.authorizer_filtered(Some(principal))
    }

    fn authorizer_filtered(&self, principal: Option<&PrincipalId>) -> Result<PolicyAuthorizer> {
        let version = self.policy_version()?;
        let mut authorizer = PolicyAuthorizer::new(version);
        if let Some((_, true)) = self.policy_state()? {
            authorizer.revoke();
        }
        if version == 0 {
            return Ok(authorizer);
        }
        // 两条静态 SQL 分别保留全策略兼容和主体主键前缀查找，避免 OR 条件退化为全表扫描。
        let sql = if principal.is_some() {
            "SELECT principal_id, permission, scope_id, policy_version FROM grants WHERE principal_id=?1"
        } else {
            "SELECT principal_id, permission, scope_id, policy_version FROM grants"
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(principal.into_iter().map(PrincipalId::as_str)),
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )?;
        for row in rows {
            let (principal, permission, scope, policy_version) = row?;
            let Ok(principal) = PrincipalId::new(principal) else {
                continue;
            };
            let Ok(scope) = ScopeId::new(scope) else {
                continue;
            };
            let Some(permission) = parse_permission(&permission) else {
                continue;
            };
            authorizer.grant_at_version(principal, permission, scope, policy_version.max(0) as u64);
        }
        Ok(authorizer)
    }

    /// Whether this principal holds scope administration on the admin scope
    /// under any grant epoch. Management operations (publish/revoke) validate
    /// against this instead of the live authorizer, so a revoked policy can
    /// still be republished by its administrator (SC-04 break-glass).
    /// 判断恢复管理员的历史授权，不代替实时业务权限检查。
    /// 参数：principal：待检查的真实主体。
    /// 返回：任意已持久 epoch 中是否有 diskgraph-admin 的 scope:admin 授权；忽略当前策略撤销，供管理恢复。
    pub fn holds_admin(&self, principal: &PrincipalId) -> Result<bool> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM grants
             WHERE principal_id = ?1 AND permission = 'scope:admin'
               AND scope_id = 'diskgraph-admin'",
            [principal.as_str()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Every stored grant, for epoch renewal at bootstrap.
    /// 管理真实策略 epoch、授权与撤销；查询依实际持久状态判断。
    /// 参数：无额外输入；实例方法使用当前连接/记录。
    /// 返回：`Result<Vec<Grant>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn all_grants(&self) -> Result<Vec<Grant>> {
        let mut statement = self
            .connection
            .prepare("SELECT principal_id, permission, scope_id, policy_version FROM grants")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut grants = Vec::new();
        for row in rows {
            let (principal, permission, scope, version) = row?;
            let Some(permission) = parse_permission(&permission) else {
                continue;
            };
            let Ok(principal) = PrincipalId::new(principal) else {
                continue;
            };
            let Ok(scope) = ScopeId::new(scope) else {
                continue;
            };
            grants.push(Grant {
                principal,
                permission,
                scope,
                policy_version: version.max(0) as u64,
            });
        }
        Ok(grants)
    }
}

#[cfg(test)]
#[path = "control_query_cache_tests.rs"]
mod control_query_cache_tests;

#[path = "live_identity_permission.rs"]
mod live_identity_permission;
