//! 长连接实时权限的有界控制观察。
use crate::{Engine, EngineError, admin_scope};
use diskgraph_core::{Permission, PrincipalId};
use std::time::Instant;
impl Engine {
    /// 检查主体的候选权限是否仍有实时grant。参数：principal、token允许的permissions及原deadline。
    /// 返回：存在未撤销范围或管理范围授权；锁/SQL未及时完成返回错误，调用方须拒绝继续连接。
    pub fn has_live_permission_until(
        &self,
        principal: &PrincipalId,
        permissions: &[Permission],
        deadline: Instant,
    ) -> Result<bool, EngineError> {
        let control =
            crate::authorization_phase_diagnostic::observe("stream_identity_lock", || {
                self.control_until(deadline)
            })?;
        crate::authorization_phase_diagnostic::observe("stream_identity_sql", || {
            control.with_read_deadline(deadline, |store| {
                Ok(store.identity_has_live_permission(principal, permissions, &admin_scope())?)
            })
        })
    }
}
