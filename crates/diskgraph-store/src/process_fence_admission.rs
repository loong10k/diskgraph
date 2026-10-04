//! 同原控制连接/fence/期限的执行准备；来源：Rust D42，不新增owner或改变旧扫描/Git接口。
use crate::control_write_deadline::ControlWriteDeadline;
use crate::{ControlStore, Result, StoreError};
use rusqlite::params;
use std::time::Instant;

impl ControlStore {
    /// 参数：真实job/owner/fence、原claim期限和原会话回调；work接收同事务实际live租约。
    /// 返回：原授权与事前raw/分配准入后的回调结果。work不得重入control或执行native I/O。
    /// 先保原EXISTS/失权拒绝，再消费会话，最后完整typed验证；预算不能覆盖真实撤权错误。
    pub fn with_job_fence_with_admission<T>(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
        deadline: Instant,
        admit: &mut dyn FnMut(u64, u64, u64) -> Result<()>,
        work: impl FnOnce(u64) -> Result<T>,
    ) -> Result<T> {
        let window = ControlWriteDeadline::new(&self.connection, deadline, None)?;
        let result = (|| {
            window.begin()?;
            let connection = &self.connection;
            // 与旧fence同一EXISTS：当前owner/租约/取消/scope/IndexWrite先决定，不被Session覆写。
            let valid: bool = window.statement(|| Ok(connection.query_row(
                "SELECT EXISTS (SELECT 1 FROM jobs j JOIN scopes s ON s.scope_id = j.scope_id WHERE j.job_id = ?1 AND j.owner = ?2 AND j.fencing_token = ?3 AND j.state = 'running' AND j.cancel_requested = 0 AND j.lease_expires_unix_ms > ?4 AND s.revoked = 0 AND (NOT EXISTS (SELECT 1 FROM policy WHERE id = 1) OR EXISTS (SELECT 1 FROM policy p JOIN grants g ON g.policy_version = p.version WHERE p.id = 1 AND p.revoked = 0 AND g.principal_id = j.principal AND g.scope_id = j.scope_id AND g.permission = 'index:write')))",
                params![job_id,owner,fence as i64,ControlStore::now_ms() as i64], |r|r.get(0),
            )?))?;
            if !valid {
                return Err(StoreError::StaleOwner);
            }
            let raw = window
                .statement(|| crate::process_fence_authorization::precheck(connection, job_id))?;
            // 纯借用授权扫描也计原始字节；不先复制敏感字段，再事后补扣。
            admit(
                u64::try_from(raw).map_err(|_| StoreError::BudgetExceeded)?,
                1,
                0,
            )?;
            window.statement(|| {
                crate::job_authority_gate::validate_job_with_admission(
                    connection,
                    job_id,
                    &[diskgraph_core::Permission::IndexWrite],
                    false,
                    admit,
                )
            })?;
            let lease: i64 = window.statement(|| {
                Ok(connection.query_row(
                    "SELECT lease_expires_unix_ms FROM jobs WHERE job_id=?1",
                    [job_id],
                    |r| r.get(0),
                )?)
            })?;
            admit(8, 1, 0)?;
            let lease = u64::try_from(lease)
                .map_err(|_| StoreError::InvalidGraph("invalid job lease".into()))?;
            let value = work(lease)?;
            // work可能已实际提交图库；本控制事务失败不声称跨库原子，调用方以唯一回执恢复。
            window.commit()?;
            Ok(value)
        })();
        window.finish(result)
    }
}
