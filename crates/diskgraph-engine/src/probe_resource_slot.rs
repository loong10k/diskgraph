use crate::live_evidence::git_private_directory_owner::GitPrivateDirectoryOwner;
use crate::scan_worker_registry::ScanWorkerRegistry;
use std::sync::Arc;

/// 固定session槽及独占目录payload；来源：PF-06同owner恢复，不建立动态目录队列。
pub(crate) struct ProbeResourceSlot {
    pub(crate) inner: Arc<ScanWorkerRegistry>,
    pub(crate) generation: u64,
    pub(crate) reserved: bool,
    pub(crate) session_alive: bool,
    pub(crate) directory_borrowed: bool,
    pub(crate) draining: bool,
    pub(crate) directory: Option<GitPrivateDirectoryOwner>,
}
