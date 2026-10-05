use crate::{worker_limits::WorkerLimits, worker_scan_request::WorkerScanRequest};
use serde::Deserialize;

/// 单次 helper 执行输入与后续取消命令；来源：PF-06 v2，不改变旧 Frame::Request 的形状。
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WorkerRequest {
    Request {
        version: u32,
        request: WorkerScanRequest,
        limits: WorkerLimits,
    },
    Cancel {},
}
