//! 持久请求身份接线；来源：DiskGraph 原生 Rust SC-06，不引入独立调度或授权 owner。

use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, JobRequestAuthority, ScopeId};
use diskgraph_store::{JobKind, JobRecord};

impl Engine {
    /// 使用适配器已验证的不可变请求身份创建索引任务。
    /// 参数：scope_id 为实际范围，authority 为请求来源与能力上限，authorizer 为当前请求授权。
    /// 返回：持久任务或入队授权、容量及存储错误；不接受客户端 JSON 权限。
    pub fn index_scope_with_authority(
        &self,
        scope_id: &ScopeId,
        authority: &JobRequestAuthority,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        self.create_scan_job(scope_id, JobKind::Index, authority, authorizer)
    }

    /// 使用适配器已验证的不可变请求身份创建重扫任务。
    /// 参数：scope_id 为实际范围，authority 为原始请求期限和能力，authorizer 为当前请求授权。
    /// 返回：持久任务或授权、容量及存储错误；重连不得修改已存在任务的期限。
    pub fn sync_scope_with_authority(
        &self,
        scope_id: &ScopeId,
        authority: &JobRequestAuthority,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        self.create_scan_job(scope_id, JobKind::Sync, authority, authorizer)
    }

    /// 严格执行持久来源明确的任务，缺少来源的旧任务不能隐式视为本机请求。
    /// 参数：job_id 为指定任务，owner 为本次执行 owner。
    /// 返回：真实终态或认领、权限及执行失败；不自动补充本机权限。
    pub fn run_job_strict(&self, job_id: &str, owner: &str) -> Result<JobRecord, EngineError> {
        self.run_job_with_authority_mode(job_id, owner, true)
    }
}

/// 读取真实 Unix 秒；参数：无。返回：当前秒或时钟无法表达时拒绝授权。
pub(super) fn unix_seconds() -> Result<u64, BusinessError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| BusinessError::PermissionDenied)
}

/// 图库事务内的纯末检，不获取控制锁或刷新请求期限。
/// 参数：authority 为原始请求身份，cancel 为本代取消，lease_expires 为写锁内读取的真实租约，started/max_duration_ms 为整次扫描期限。
/// 返回：可提交，或权限到期、取消、租约/扫描期限失败。
pub(super) fn check_scan_commit(
    authority: Option<&JobRequestAuthority>,
    cancel: &std::sync::atomic::AtomicBool,
    lease_expires: u64,
    started: std::time::Instant,
    max_duration_ms: u64,
) -> diskgraph_store::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| diskgraph_store::StoreError::Conflict("request clock unavailable".into()))?;
    if let Some(authority) = authority {
        authority.validate_at(now.as_secs()).map_err(|_| {
            diskgraph_store::StoreError::Conflict("request authority expired before commit".into())
        })?;
    }
    if now.as_millis() >= u128::from(lease_expires) {
        return Err(diskgraph_store::StoreError::StaleOwner);
    }
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(diskgraph_store::StoreError::Conflict(
            "scan cancelled before commit".into(),
        ));
    }
    if started.elapsed() > std::time::Duration::from_millis(max_duration_ms) {
        return Err(diskgraph_store::StoreError::BudgetExceeded);
    }
    Ok(())
}
