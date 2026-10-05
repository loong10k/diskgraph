/// 父驱动的唯一平台进程 owner；Linux 明确选择原子 pidfd，不回退 Unix/SCM。
/// 来源：原生 Rust PF-06 物理扫描进程合同，无 Java 对等对象。
#[cfg(target_os = "linux")]
pub(super) use crate::native_child::LinuxAtomicChild as ScanWorkerChild;

/// macOS 父驱动的 retained-leader owner；来源：原生 Rust PF-06 平台进程合同。
#[cfg(target_os = "macos")]
pub(super) use crate::native_child::UnixChild as ScanWorkerChild;

/// Windows 父驱动的唯一 Job/进程 owner；来源：原生 Rust PF-06 平台进程合同。
#[cfg(windows)]
pub(super) use crate::native_child::WindowsChild as ScanWorkerChild;
