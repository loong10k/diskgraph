//! 临时借出槽的归还守卫，panic/到期也必须把原 owner 放回原位置。
use super::ScanWorkerRegistry;
use crate::scan_worker_child::ScanWorkerChild;
use crate::scan_worker_owner_slot::ScanWorkerOwnerSlot;

/// 锁外处置期间的唯一 owner；来源：原生 Rust PF-06，无 Java 对等对象。
pub(super) struct ScanWorkerDrainOwner<'a> {
    registry: &'a ScanWorkerRegistry,
    index: usize,
    owner: Option<ScanWorkerChild>,
}
impl<'a> ScanWorkerDrainOwner<'a> {
    /// 参数：registry/index 为已置 Draining 的原槽；返回：同一 child 的归还守卫。
    pub(super) fn new(
        registry: &'a ScanWorkerRegistry,
        index: usize,
        owner: ScanWorkerChild,
    ) -> Self {
        Self {
            registry,
            index,
            owner: Some(owner),
        }
    }
    /// 参数：无；返回：原 child 独占借用，不复制任何内核句柄或 pending 内存。
    pub(super) fn owner(&mut self) -> &mut ScanWorkerChild {
        self.owner.as_mut().expect("unique draining owner")
    }
    /// 参数：无；返回：无；仅实际 Complete 后调用，释放原槽及已完成 owner。
    pub(super) fn complete(mut self) {
        let owner = self.owner.take();
        {
            let mut slots = self
                .registry
                .slots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slots[self.index] = ScanWorkerOwnerSlot::Vacant;
        }
        // 已完成 child 的析构也在 registry 状态锁外。
        drop(owner);
    }
}
impl Drop for ScanWorkerDrainOwner<'_> {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.take() {
            // 归还不能因 try_lock 失败丢 owner；短状态锁内无 OS 等待或用户回调。
            let mut slots = self
                .registry
                .slots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slots[self.index] = ScanWorkerOwnerSlot::Retained(owner);
        }
    }
}
