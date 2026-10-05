use crate::{AuthorizationWithdrawalWatch, ControlStore, Result, StoreError};
use diskgraph_core::{Permission, PrincipalId, ScopeId};

impl ControlStore {
    /// 在实际 scope 已解析、首次授权前订阅请求依赖；调用方须置于原控制读取预算内。
    /// 参数：principal/scope/permission 是本请求实际授权依赖，不是显示 scope 或客户端授予信息。
    /// 返回：可靠原生身份上的负向 watch，未知能力为 None；SQL/容量错误保持原 StoreError。
    pub fn watch_authorization_withdrawal(
        &self,
        principal: &PrincipalId,
        scope: &ScopeId,
        permission: &Permission,
    ) -> Result<Option<AuthorizationWithdrawalWatch>> {
        if self.withdrawal_incarnation.identity().is_none() {
            return Ok(None);
        }
        // 无持久策略的可信兼容调用只依赖 scope；不把不存在或已经撤销的 grant 当作 Allow。
        let (version, revoked, generation): (Option<i64>, Option<bool>, i64) =
            self.connection.query_row(
                "SELECT p.version,p.revoked,a.generation FROM authorization_state a
             LEFT JOIN policy p ON p.id=1 WHERE a.id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let epoch = match (version, revoked) {
            (Some(version), Some(false)) => Some(version.max(0) as u64),
            (Some(_), Some(true)) | (None, None) => None,
            _ => {
                return Err(StoreError::InvalidGraph(
                    "inconsistent withdrawal policy snapshot".into(),
                ));
            }
        };
        let generation = u64::try_from(generation)
            .map_err(|_| StoreError::InvalidGraph("invalid authorization generation".into()))?;
        crate::withdrawal_registry::register(self, principal, scope, permission, epoch, generation)
    }
}

/// 在撤权事务内读取已有触发器最终生成的计数，只用于同一可靠文件内的提交排序。
/// 参数：connection 为仍未提交的原事务连接；返回：可验证非负计数或原数据库错误。
pub(crate) fn transaction_generation(connection: &rusqlite::Connection) -> Result<u64> {
    let generation: i64 = connection.query_row(
        "SELECT generation FROM authorization_state WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    u64::try_from(generation)
        .map_err(|_| StoreError::InvalidGraph("invalid authorization generation".into()))
}
