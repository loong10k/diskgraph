//! Windows 同一绝对期限的单次恢复扫描，原 cleanup/wait 始终在状态锁外。
use super::ScanWorkerRegistry;
use super::windows_registry_owner::WindowsRegistryOwner;
use crate::EngineError;
use crate::native_child::CleanupProgress;
use crate::scan_worker_error_projection::ScanWorkerErrorProjection;
use crate::scan_worker_owner_slot::ScanWorkerOwnerSlot;
use std::sync::TryLockError;
use std::time::Instant;

impl ScanWorkerRegistry {
    /// 参数：deadline 为调用者原绝对期限；返回：true 仅实际全部空槽。
    /// Pending/锁竞争/到期返回 false；原 native 错误返回 Err，原 owner 回同槽。
    /// OS 单调用及归还 owner 所需短状态锁不承诺硬墙钟上限；没有内部重试等待。
    pub(crate) fn drain_until(&self, deadline: Instant) -> Result<bool, EngineError> {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        let length = match self.slots.try_lock() {
            Ok(slots) => slots.len(),
            Err(TryLockError::WouldBlock) => return Ok(false),
            Err(TryLockError::Poisoned(_)) => return Err(EngineError::Poisoned),
        };
        let mut first_error = None;
        for index in 0..length {
            if Instant::now() >= deadline {
                break;
            }
            let owner = {
                let mut slots = match self.slots.try_lock() {
                    Ok(slots) => slots,
                    Err(TryLockError::WouldBlock) => break,
                    Err(TryLockError::Poisoned(_)) => {
                        if first_error.is_none() {
                            first_error = Some(EngineError::Poisoned);
                        }
                        break;
                    }
                };
                // 抢到锁不代表仍在期限内；未取 owner 时可以直接返回 Pending。
                if Instant::now() >= deadline {
                    break;
                }
                if matches!(slots[index], ScanWorkerOwnerSlot::Retained(_)) {
                    match std::mem::replace(&mut slots[index], ScanWorkerOwnerSlot::Draining) {
                        ScanWorkerOwnerSlot::Retained(owner) => Some(owner),
                        _ => unreachable!("retained slot observed under the same lock"),
                    }
                } else {
                    None
                }
            };
            if let Some(owner) = owner {
                let mut pending = WindowsRegistryOwner::new(self, index, owner);
                match pending.owner().poll_cleanup(deadline) {
                    Ok(CleanupProgress::Complete) => pending.complete(),
                    Ok(CleanupProgress::Pending) => drop(pending),
                    Err(error) => {
                        // 先归还唯一 owner，再投影错误；投影失败也不影响原责任。
                        drop(pending);
                        if first_error.is_none() {
                            first_error = Some(ScanWorkerErrorProjection::child(error));
                        }
                    }
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        match self.slots.try_lock() {
            Ok(slots) => Ok(slots
                .iter()
                .all(|slot| matches!(slot, ScanWorkerOwnerSlot::Vacant))),
            Err(TryLockError::WouldBlock) => Ok(false),
            Err(TryLockError::Poisoned(_)) => Err(EngineError::Poisoned),
        }
    }
}
