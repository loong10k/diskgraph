//! 监督槽的可信 held 文件预留；尚未接入受信命名空间或实际监督进程。
mod active_slot;
mod slot_error;
mod slot_reservation;
pub use active_slot::ActiveSlot;
pub use slot_error::SlotError;
pub use slot_reservation::SlotReservation;

#[cfg(target_os = "linux")]
mod linux_supervisor_namespace;
#[cfg(target_os = "linux")]
pub use linux_supervisor_namespace::LinuxSupervisorNamespace;

#[cfg(target_os = "linux")]
mod linux_supervisor_trust;
#[cfg(target_os = "linux")]
pub use linux_supervisor_trust::LinuxSupervisorTrust;

#[cfg(target_os = "linux")]
mod linux_supervisor_peer;
#[cfg(target_os = "linux")]
pub use linux_supervisor_peer::LinuxSupervisorPeer;
#[cfg(target_os = "linux")]
mod linux_supervisor_materials;
#[cfg(target_os = "linux")]
pub use linux_supervisor_materials::LinuxSupervisorMaterials;

#[cfg(all(test, target_os = "linux"))]
mod linux_supervisor_materials_tests;

#[cfg(all(test, target_os = "linux"))]
mod linux_supervisor_namespace_tests;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod active_slot_tests;
