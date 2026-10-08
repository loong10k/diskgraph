//! 扫描原 fence 的有期限控制事务，不使用 Process 专用预检查。
use crate::control_write_deadline::ControlWriteDeadline;
use crate::{ControlStore, Result, StoreError};
use diskgraph_core::Permission;
use rusqlite::params;
use std::time::Instant;

impl ControlStore {
    /// 在扫描原始期限内复验真实 owner、租约、取消、范围和持久请求权限。
    /// 参数：job_id/owner/fence 为原代次，deadline 为原期限，work 为不得重入 control 的内部回调。
    /// 返回：原 fence 校验后的结果；锁竞争/SQL/提交不得刷新期限，失效 owner 保留 StaleOwner。
    /// work 若提交独立图库，后续控制提交失败不能回滚图库，须按原发布回执恢复。
    pub fn with_job_fence_until<T>(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        deadline: Instant,
        work: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let window = ControlWriteDeadline::new(&self.connection, deadline, None)?;
        let result = (|| {
            window.begin()?;
            let valid: bool = window.statement(|| Ok(self.connection.query_row(
                "SELECT EXISTS (SELECT 1 FROM jobs j JOIN scopes s ON s.scope_id = j.scope_id WHERE j.job_id = ?1 AND j.owner = ?2 AND j.fencing_token = ?3 AND j.state = 'running' AND j.cancel_requested = 0 AND j.lease_expires_unix_ms > ?4 AND s.revoked = 0 AND (NOT EXISTS (SELECT 1 FROM policy WHERE id = 1) OR EXISTS (SELECT 1 FROM policy p JOIN grants g ON g.policy_version = p.version WHERE p.id = 1 AND p.revoked = 0 AND g.principal_id = j.principal AND g.scope_id = j.scope_id AND g.permission = 'index:write')))",
                params![job_id, owner, fence as i64, Self::now_ms() as i64], |row| row.get(0),
            )?))?;
            if !valid {
                return Err(StoreError::StaleOwner);
            }
            window.statement(|| {
                crate::job_authority_gate::validate_job(
                    &self.connection,
                    job_id,
                    &[Permission::IndexWrite],
                    false,
                )
            })?;
            window.check()?;
            let value = work()?;
            window.check()?;
            window.commit()?;
            Ok(value)
        })();
        window.finish(result)
    }
}
