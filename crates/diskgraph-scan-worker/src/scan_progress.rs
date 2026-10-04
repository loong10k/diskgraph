use diskgraph_disktree_core::scan::ScanSnapshot;
use serde::{Deserialize, Serialize};

/// 实际扫描进度全字段；来源：Engine scan_execution 保存的 pinned ScanSnapshot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanProgress {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub errors: u64,
    pub finished: bool,
    pub cancelled: bool,
    pub messages: Vec<String>,
}

impl ScanProgress {
    /// 转移上游快照。参数为真实 snapshot；返回保留错误明细与所有状态的记录。
    /// 参数：snapshot 为真实 pinned ScanSnapshot。
    /// 返回：保留所有计数、状态和错误 messages 的传输记录。
    pub fn from_native(snapshot: ScanSnapshot) -> Self {
        Self {
            files: snapshot.files,
            dirs: snapshot.dirs,
            bytes: snapshot.bytes,
            errors: snapshot.errors,
            finished: snapshot.finished,
            cancelled: snapshot.cancelled,
            messages: snapshot.messages,
        }
    }

    /// 恢复原快照。返回真实字段；finished 不代表 OS wait 或物理退场许可。
    /// 参数：self 为完整的进度传输记录。
    /// 返回：原 ScanSnapshot；finished 不表示 OS 进程退出。
    pub fn into_native(self) -> ScanSnapshot {
        ScanSnapshot {
            files: self.files,
            dirs: self.dirs,
            bytes: self.bytes,
            errors: self.errors,
            finished: self.finished,
            cancelled: self.cancelled,
            messages: self.messages,
        }
    }
}
