use crate::{FlatNode, ScanProgress, ScanRequest};
use serde::{Deserialize, Serialize};

/// 版本化平铺帧；来源：PF-06。Hello 是声明而不是可执行文件完整性验证。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    Hello {
        version: u32,
        target: String,
        pin: String,
    },
    Request {
        request: ScanRequest,
    },
    Progress {
        progress: ScanProgress,
    },
    Node {
        node: FlatNode,
    },
    End {
        nodes: u64,
    },
    Error {
        code: String,
        message: String,
    },
    #[serde(deserialize_with = "crate::cancel_body::CancelBody::deserialize_unit")]
    Cancel,
}
