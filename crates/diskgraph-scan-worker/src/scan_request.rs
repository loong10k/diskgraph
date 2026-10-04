use crate::{NativePath, ScanOptions};
use serde::{Deserialize, Serialize};

/// helper 的纯扫描输入；来源：PF-06 原生协议，不包含数据库或主体授权信息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub root: NativePath,
    pub options: ScanOptions,
}
