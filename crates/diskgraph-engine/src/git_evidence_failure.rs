//! Git 任务固定失败分类与授权读出；来源：原生 Rust EC-02 / SC-06。
//! 不解析错误字符串，也不持久化路径、工具输出或原始数据库诊断。
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, GitEvidenceFailure, GitEvidenceFailureCode, GitEvidenceFailurePhase,
    Permission, PrincipalId,
};
use diskgraph_store::{JobState, StoreError};

/// 参数：error 为真实类型化执行结果，state 为持久取消事实选择的终态。
/// 返回：有限业务分类；fence 的复合拒绝保持 Conflict，不猜测是哪一种权限/owner 变化。
pub(super) fn execution(error: &EngineError, state: JobState) -> GitEvidenceFailure {
    let code = if state == JobState::Cancelled {
        GitEvidenceFailureCode::Cancelled
    } else {
        match error.primary() {
            EngineError::Business(
                BusinessError::BudgetExceeded | BusinessError::ResourceExhausted,
            )
            | EngineError::Store(StoreError::BudgetExceeded) => {
                GitEvidenceFailureCode::BudgetExceeded
            }
            EngineError::Business(BusinessError::Timeout) => GitEvidenceFailureCode::Timeout,
            EngineError::Business(
                BusinessError::PermissionDenied | BusinessError::ApprovalRequired,
            ) => GitEvidenceFailureCode::PermissionDenied,
            EngineError::Business(
                BusinessError::Conflict
                | BusinessError::StalePlan
                | BusinessError::IdempotencyConflict,
            )
            | EngineError::Store(
                StoreError::Conflict(_) | StoreError::StaleOwner | StoreError::IdempotencyConflict,
            ) => GitEvidenceFailureCode::Conflict,
            EngineError::Business(BusinessError::Unsupported)
            | EngineError::Store(
                StoreError::UnsupportedLocator(_) | StoreError::UnsupportedSchema(_),
            ) => GitEvidenceFailureCode::Unsupported,
            EngineError::Business(BusinessError::Unavailable)
            | EngineError::Io(_)
            | EngineError::Store(StoreError::Io(_)) => GitEvidenceFailureCode::Unavailable,
            _ => GitEvidenceFailureCode::InternalError,
        }
    };
    GitEvidenceFailure::new(GitEvidenceFailurePhase::Execution, code)
}

impl Engine {
    /// 按实际任务 scope 读取安全失败诊断，供断线重连查询。
    /// 参数：job_id 为任务，principal/authorizer 为当前请求身份。
    /// 返回：固定阶段/代码或无失败；撤销范围及 OperationView 缺失均拒绝。
    pub fn git_job_failure(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Option<GitEvidenceFailure>, EngineError> {
        let control = self.control()?;
        let job = control.job(job_id)?;
        if control.scope(&job.scope_id)?.revoked {
            return Err(BusinessError::PermissionDenied.into());
        }
        Self::require_with_control(
            &control,
            authorizer,
            principal,
            &Permission::OperationView,
            &job.scope_id,
        )?;
        Ok(control.git_job_failure(job_id)?)
    }
}
