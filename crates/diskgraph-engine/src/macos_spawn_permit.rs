use crate::macos_installation_lease::MacosInstallationLease;
use crate::macos_installation_lock::MacosInstallationLock;
use std::sync::Arc;

/// 持永久更新锁的单次出生材料；镜像与更新守卫一起移动，不能单独缓存旧资格。
/// 来源：原生 Rust PF-06 macOS安装更新协调合同；无 Java 对等对象。
pub(super) struct MacosSpawnPermit {
    installation: Arc<MacosInstallationLease>,
    update_lock: MacosInstallationLock,
}
impl MacosSpawnPermit {
    /// 参数：installation已在update_lock内重新核验；返回：单次可移动材料，不复制锁FD。
    pub(super) fn new(
        installation: Arc<MacosInstallationLease>,
        update_lock: MacosInstallationLock,
    ) -> Self {
        Self {
            installation,
            update_lock,
        }
    }
    /// 参数：消费自身；返回：原image lease与原锁守卫，启动器必须保留至C出生返回：。
    pub(super) fn into_parts(self) -> (Arc<MacosInstallationLease>, MacosInstallationLock) {
        (self.installation, self.update_lock)
    }
}
