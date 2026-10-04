//! 原生有类型失败到原业务错误的映射；来源：Rust D42，正文从执行模块等价移动。
use crate::EngineError;
use diskgraph_core::{BusinessError, ProcessEvidenceFailureCode as Code};

/// 参数：原生有类型失败；返回：对应业务类别，不从错误字符串猜测身份或授权。
pub(super) fn native_error(code: Code) -> EngineError {
    match code {
        Code::BudgetExceeded => BusinessError::BudgetExceeded,
        Code::Timeout => BusinessError::Timeout,
        Code::Cancelled | Code::Conflict => BusinessError::Conflict,
        Code::PermissionDenied => BusinessError::PermissionDenied,
        Code::Unsupported => BusinessError::Unsupported,
        Code::Unavailable => BusinessError::Unavailable,
        Code::InternalError => BusinessError::InternalError,
    }
    .into()
}
