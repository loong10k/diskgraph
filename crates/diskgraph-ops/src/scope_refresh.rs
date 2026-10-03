//! scope_refresh：既有文件操作职责的原生 Rust 实现。
use crate::job_outcome::JobOutcome;
use crate::ops_error::OpsError;
use diskgraph_core::PrincipalId;
use diskgraph_core::ScopeId;
use diskgraph_engine::Engine;

/// 操作后重建范围的只读快照。
/// 参数：engine、scope_id、principal 指定刷新授权。
/// 返回：实际刷新 job_id 和 JobState；授权、入队或执行失败返回错误。
/// Refreshes the index for the scope an operation touched, so a later query
/// reflects the file system instead of the plan's snapshot (OP-10).
pub fn refresh_scope_after_operation(
    engine: &std::sync::Arc<Engine>,
    scope_id: &ScopeId,
    principal: &PrincipalId,
) -> Result<JobOutcome, OpsError> {
    let authorizer = engine
        .policy_authorizer()
        .map_err(|error| OpsError::Stale(error.to_string()))?;
    let job = engine
        .index_scope(scope_id, principal, &authorizer)
        .map_err(|error| OpsError::Stale(error.to_string()))?;
    let finished = engine
        .run_job(&job.job_id, "ops-refresh")
        .map_err(|error| OpsError::Stale(error.to_string()))?;
    Ok(JobOutcome {
        job_id: finished.job_id,
        state: finished.state,
    })
}
