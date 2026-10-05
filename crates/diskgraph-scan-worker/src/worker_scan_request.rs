use crate::{ScanOptions, worker_path::WorkerPath};
use serde::{Deserialize, Serialize};

/// 执行 v2 的严格扫描参数；来源：pinned ScanOptions 与 PF-06，不包含主体/数据库字段。
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerScanRequest {
    pub(crate) root: WorkerPath,
    pub(crate) options: ScanOptions,
}
