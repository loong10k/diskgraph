use crate::{Engine, ScanWorkerRecovery};
use std::sync::Arc;

/// executor地址空间中原Engine与恢复责任；来源：PF-06 guardian/executor合同，无Java对等对象。
/// 不持容量槽，不序列化或复制owner；绑定拒绝时整体返回原对象。
#[must_use = "retain original executor resources on bind rejection"]
pub struct ExecutorParts {
    /// 原Engine，不能用重建实例替代pending资源的持有者。
    pub engine: Arc<Engine>,
    /// 原Engine扫描宿主同一资源表的恢复责任。
    pub scan: Option<ScanWorkerRecovery>,
    /// 原Engine Windows探针宿主同一资源表的恢复责任。
    #[cfg(windows)]
    pub probe: Option<crate::ProbeRecovery>,
}
