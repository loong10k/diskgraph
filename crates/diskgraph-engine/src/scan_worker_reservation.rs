#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_child::ScanWorkerChild;
use crate::scan_worker_registry::ScanWorkerRegistry;
use std::cell::Cell;
use std::sync::Arc;

/// 从出生前持续到原child实际wait或转入Retained的唯一槽预留，不授予执行权限。
/// 来源：原生 Rust PF-06有限宿主容量；Drop仅在仍持有本预留的释放责任时释放槽。
pub(super) struct ScanWorkerReservation {
    registry: Arc<ScanWorkerRegistry>,
    index: usize,
    release_on_drop: Cell<bool>,
}

impl ScanWorkerReservation {
    /// 参数：registry/index为已真实预留的槽；返回：唯一预留，没有新child。
    pub(super) fn new(registry: Arc<ScanWorkerRegistry>, index: usize) -> Self {
        Self {
            registry,
            index,
            release_on_drop: Cell::new(true),
        }
    }
    /// 参数：owner为本次失败移交的唯一child；返回：owner与释放责任交给registry，后续Drop不释放该索引。
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    pub(super) fn retain(&self, owner: ScanWorkerChild) {
        self.registry.retain(self.index, owner);
        // owner移交后释放责任属于registry；旧预留不能清理回收后复用的同索引槽。
        self.release_on_drop.set(false);
    }
}

impl Drop for ScanWorkerReservation {
    fn drop(&mut self) {
        if self.release_on_drop.get() {
            self.registry.release(self.index);
        }
    }
}
