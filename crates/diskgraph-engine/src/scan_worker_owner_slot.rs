#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_child::ScanWorkerChild;

/// 一个原有限槽的状态；Retained始终唯一拥有仍需真实回收的child。
/// 来源：原生 Rust PF-06 出生前准入和显式恢复句柄，不拥有数据库或授权。
pub(super) enum ScanWorkerOwnerSlot {
    Vacant,
    #[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
    Reserved,
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    Draining,
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    Retained(ScanWorkerChild),
}
