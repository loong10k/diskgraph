use serde::{Deserialize, Serialize};
/// 固定原生占用观察方法；来源：原生 Rust D42 / EC-04，名称不是平台能力认证。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessObservationMethod {
    MacLibprocV1,
    LinuxProcfsV1,
    WindowsRestartManagerV1,
}
impl ProcessObservationMethod {
    /// 参数：无；返回：固定可持久方法标签。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MacLibprocV1 => "mac_libproc_v1",
            Self::LinuxProcfsV1 => "linux_procfs_v1",
            Self::WindowsRestartManagerV1 => "windows_restart_manager_v1",
        }
    }
    /// 参数：扫描历代身份；返回：与方法平台相符且身份已验证的结构资格。
    pub fn accepts_epoch(self, epoch: &crate::IndexedFileEpoch) -> bool {
        epoch.validate().is_ok()
            && matches!(
                (self, epoch),
                (
                    Self::MacLibprocV1,
                    crate::IndexedFileEpoch::MacGeneration { .. }
                ) | (
                    Self::LinuxProcfsV1,
                    crate::IndexedFileEpoch::LinuxHandle { .. }
                ) | (
                    Self::WindowsRestartManagerV1,
                    crate::IndexedFileEpoch::Windows { .. }
                )
            )
    }
}
