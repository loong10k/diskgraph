use crate::recovery_slot::ActiveSlot;
use crate::{Engine, ScanWorkerRecovery};
use std::sync::Arc;
/// 监督绑定的原唯一材料；来源：PF-06 原 owner 合同，无 Java 对等对象。
/// 错配时整体返回；调用方必须保留原槽与原 Recovery，不能用新池替代。
#[must_use = "retain all original supervisor parts on bind rejection"]
pub struct SupervisorParts {
    /// 原 Engine，初始化前已取得监督槽；不从远程请求创建此材料。
    pub engine: Arc<Engine>,
    /// 与原 Engine 扫描宿主同一资源表的外部恢复责任。
    pub scan: Option<ScanWorkerRecovery>,
    /// 原 Windows 探针资源池的外部恢复责任。
    #[cfg(windows)]
    pub probe: Option<crate::ProbeRecovery>,
    /// 原已同步 ACTIVE 的容量槽，公开接口不能清除。
    pub slot: ActiveSlot,
}
