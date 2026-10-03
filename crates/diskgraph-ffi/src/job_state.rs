use crate::job_authorization::JobAuthorization;
use serde_json::Value;

/// 作业线程与句柄共享的完成结果、进度和实时授权状态。
/// 来源：DiskGraph 原生 Rust FFI 作业状态；无 Java 对应对象。
pub(crate) struct JobState {
    pub(crate) finished: bool,
    pub(crate) result: Option<Result<serde_json::Value, String>>,
    pub(crate) progress: Option<Value>,
    pub(crate) authorization: Option<JobAuthorization>,
}
