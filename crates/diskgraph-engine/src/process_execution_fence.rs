//! 执行线程的同账本事务复验；来源：Rust D42，不创建控制连接或独立授权 owner。
use crate::native_process::ProcessNativeSession;
use crate::process_native_error::native_error;
use crate::{Engine, EngineError};
use diskgraph_store::StoreError;
use std::time::Instant;

/// 参数：原引擎/job/owner/fence、绝对期限、会话和纯控制事务工作；返回：同事务实际 lease 下的工作结果。
/// 拒权优先沿 Store 原复合 fence/借用权限预检；完整 typed 解码仍在原会话拥有前收费。
pub(super) fn checked<T>(
    engine: &Engine,
    job: &str,
    owner: &str,
    fence: u64,
    deadline: Instant,
    session: &ProcessNativeSession<'_>,
    work: impl FnOnce(u64) -> diskgraph_store::Result<T>,
) -> Result<T, EngineError> {
    // 原 exp 是不可变已认证事实。即使执行期限先耗尽，也只作纯拒绝，不另开 SQL 查询窗口。
    // 回调成功不能放行工作；真实 owner、live grants 与同一期限仍由下面原事务检查。
    if session.request_authority_denied() {
        return Err(StoreError::Conflict("job request authority denied".into()).into());
    }
    let mut control = loop {
        if let Some(control) = engine.try_control_store()? {
            break control;
        }
        if Instant::now() >= deadline {
            session.check().map_err(native_error)?;
            return Err(StoreError::BudgetExceeded.into());
        }
        std::thread::yield_now();
    };
    let mut admit = |raw, entries, allocation| {
        session
            .admit(raw, entries, allocation)
            .map_err(|_| StoreError::BudgetExceeded)
    };
    let result =
        control.with_job_fence_with_admission(job, owner, fence, deadline, &mut admit, work);
    if matches!(&result, Err(StoreError::BudgetExceeded)) {
        // Store callback 的有限 carrier 不改变会话已锁存的 Timeout/Cancelled/资源类别。
        session.check().map_err(native_error)?;
    }
    result.map_err(EngineError::from)
}
