use crate::EngineError;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_child::ScanWorkerChild;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_error_projection::ScanWorkerErrorProjection;
use crate::scan_worker_owner_slot::ScanWorkerOwnerSlot;
#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
use crate::scan_worker_reservation::ScanWorkerReservation;
use diskgraph_core::BusinessError;
use std::sync::{Arc, Mutex};

/// Recovery与服务引用同一个固定容量表，槽内owner从不Clone或交给隐藏后台reaper。
/// 来源：原生 Rust PF-06；互斥锁只覆盖状态移动，OS cleanup/wait永远在锁外。
pub(super) struct ScanWorkerRegistry {
    slots: Mutex<Vec<ScanWorkerOwnerSlot>>,
}

impl ScanWorkerRegistry {
    /// 参数：capacity为宿主明确的非零容量；返回：已一次预留全部槽的共享状态。
    pub(super) fn new(capacity: u32) -> Result<Arc<Self>, EngineError> {
        if capacity == 0 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let length = usize::try_from(capacity).map_err(|_| BusinessError::ResourceExhausted)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(length)
            .map_err(|_| BusinessError::ResourceExhausted)?;
        slots.resize_with(length, || ScanWorkerOwnerSlot::Vacant);
        Ok(Arc::new(Self {
            slots: Mutex::new(slots),
        }))
    }

    /// 参数：self为原registry；返回：出生前独占槽或ResourceExhausted，不创建child。
    #[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
    pub(super) fn reserve(self: &Arc<Self>) -> Result<ScanWorkerReservation, EngineError> {
        let mut slots = self.slots.lock().map_err(|_| EngineError::Poisoned)?;
        let index = slots
            .iter()
            .position(|slot| matches!(slot, ScanWorkerOwnerSlot::Vacant))
            .ok_or(BusinessError::ResourceExhausted)?;
        slots[index] = ScanWorkerOwnerSlot::Reserved;
        Ok(ScanWorkerReservation::new(Arc::clone(self), index))
    }

    /// 参数：无；返回：所有非Vacant槽的标量计数，不读取或复制child。
    pub(super) fn occupied(&self) -> Result<usize, EngineError> {
        let slots = self.slots.lock().map_err(|_| EngineError::Poisoned)?;
        Ok(slots
            .iter()
            .filter(|slot| !matches!(slot, ScanWorkerOwnerSlot::Vacant))
            .count())
    }

    /// 参数：index为原预留槽；返回：释放未出生或已经在调用栈真实回收的预留，不清Retained。
    #[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
    pub(super) fn release(&self, index: usize) {
        // 此私有锁内没有外部回调。即使先前panic造成poison，恢复处置也必须保留owner。
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(slots[index], ScanWorkerOwnerSlot::Reserved) {
            slots[index] = ScanWorkerOwnerSlot::Vacant;
        }
    }

    /// 参数：index绑定原预留，owner为原唯一child；返回：无，不清理、不分配新槽。
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    pub(super) fn retain(&self, index: usize, owner: ScanWorkerChild) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // 原reservation未释放，其他reserve或drain无权使用这个Reserved槽。
        slots[index] = ScanWorkerOwnerSlot::Retained(owner);
    }

    /// 参数：无；返回：true仅所有槽为空，false表示执行栈仍持Reserved/Draining；清理原错保持。
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    pub(super) fn drain(&self) -> Result<bool, EngineError> {
        let length = self.slots.lock().map_err(|_| EngineError::Poisoned)?.len();
        let mut first_error = None;
        for index in 0..length {
            let owner = {
                let mut slots = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if matches!(slots[index], ScanWorkerOwnerSlot::Retained(_)) {
                    match std::mem::replace(&mut slots[index], ScanWorkerOwnerSlot::Draining) {
                        ScanWorkerOwnerSlot::Retained(owner) => Some(owner),
                        _ => unreachable!("retained slot observed under the same lock"),
                    }
                } else {
                    None
                }
            };
            if let Some(mut owner) = owner {
                // 先释放状态锁再执行实际终止/wait。成功才释放容量；失败移回原槽。
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    #[cfg(all(test, target_os = "macos"))]
                    registry_unwind_tests::cleanup_checkpoint();
                    owner.cleanup()
                }));
                let mut slots = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match result {
                    Ok(Ok(())) => {
                        slots[index] = ScanWorkerOwnerSlot::Vacant;
                        // 已完成 owner 的析构也不能落在状态锁内。
                        drop(slots);
                        drop(owner);
                    }
                    Ok(Err(error)) => {
                        slots[index] = ScanWorkerOwnerSlot::Retained(owner);
                        drop(slots);
                        if first_error.is_none() {
                            first_error = Some(ScanWorkerErrorProjection::child(error));
                        }
                    }
                    Err(payload) => {
                        // 展开前恢复原责任，不能让 Drop 清掉进程却留下永久 Draining。
                        slots[index] = ScanWorkerOwnerSlot::Retained(owner);
                        drop(slots);
                        std::panic::resume_unwind(payload);
                    }
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(self.occupied()? == 0)
    }

    /// 参数：无；返回：没有桌面child类型的平台仅报告真实空槽；不提供移动spawn或伪wait。
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    pub(super) fn drain(&self) -> Result<bool, EngineError> {
        Ok(self.occupied()? == 0)
    }
}

#[cfg(all(test, windows))]
#[path = "windows_registry_lock_tests.rs"]
mod windows_registry_lock_tests;

#[cfg(target_os = "macos")]
#[path = "macos_registry_drain.rs"]
mod macos_registry_drain;
#[cfg(any(windows, target_os = "macos"))]
#[path = "scan_worker_drain_owner.rs"]
mod scan_worker_drain_owner;
#[cfg(windows)]
#[path = "windows_registry_drain.rs"]
mod windows_registry_drain;

#[cfg(all(test, target_os = "macos"))]
#[path = "registry_unwind_tests.rs"]
mod registry_unwind_tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "linux_registry_deadline_tests.rs"]
mod linux_registry_deadline_tests;
