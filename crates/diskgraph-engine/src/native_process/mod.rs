//! 原生进程元数据观察；来源：Rust D42，不读取目标正文或进程 argv/env。
mod handle_reservation;
#[cfg(target_os = "linux")]
mod linux_directory;
#[cfg(target_os = "linux")]
mod linux_metadata;
#[cfg(all(test, target_os = "linux"))]
mod linux_namespace_tests;
#[cfg(target_os = "linux")]
mod linux_observer;
#[cfg(target_os = "linux")]
mod linux_open;
#[cfg(target_os = "linux")]
mod linux_pid;
#[cfg(target_os = "linux")]
mod linux_proc_root;
#[cfg(target_os = "linux")]
mod linux_scan_root;
#[cfg(target_os = "linux")]
mod linux_target;
mod process_native_session;

#[cfg(target_os = "linux")]
pub(crate) use linux_scan_root::{LinuxScanRoot, gap as linux_scan_gap};
pub use process_native_session::ProcessNativeSession;
