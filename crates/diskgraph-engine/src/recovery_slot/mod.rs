//! 监督槽的可信 held 文件预留；尚未接入受信命名空间或实际监督进程。
mod active_slot;
mod slot_error;
mod slot_reservation;
pub use active_slot::ActiveSlot;
pub use slot_error::SlotError;
pub use slot_reservation::SlotReservation;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod active_slot_tests;
