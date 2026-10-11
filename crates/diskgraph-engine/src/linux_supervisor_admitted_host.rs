use crate::ScanWorkerHost;
use crate::recovery_control::UnixControlWriter;
use crate::recovery_slot::LinuxSupervisorNamespace;
use std::time::Instant;

/// 同一 root 出生认证交付的宿主、原域及私有控制能力；来源：PF-06，无 Java 对等对象。
/// v1 不含控制能力；v2 必须保持原 writer 到实际原 owner 退休，不允许远程兼容回退。
pub struct LinuxSupervisorAdmittedHost {
    /// 已完整核验的固定 worker 原宿主。
    pub host: ScanWorkerHost,
    /// 保持原 root/user/mount 对象寿命的域。
    pub namespace: LinuxSupervisorNamespace,
    /// 原出生期限，不能因构造或角色切换刷新。
    pub deadline: Instant,
    /// v2 原私有通知写入能力；不得移到公开 stdout 或克隆。
    pub control: Option<UnixControlWriter>,
}
