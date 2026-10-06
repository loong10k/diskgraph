use crate::EngineError;
use crate::scan_worker_registry::ScanWorkerRegistry;
use std::sync::Arc;

/// 宿主必须在catch_unwind外保留的有限回收责任句柄，不Clone child或创建后台reaper。
/// 来源：原生 Rust PF-06；必须保存至所有原wait完成，主动丢最后句柄不会被Rust阻止。
#[must_use = "host must retain recovery outside catch_unwind until all children are actually waited"]
pub struct ScanWorkerRecovery {
    registry: Arc<ScanWorkerRegistry>,
}

impl ScanWorkerRecovery {
    /// 永久关闭原资源池的新准入；来源：PF-06监督退休合同。
    /// 参数：无；返回：同状态锁内关闭成功或原错误；不释放原 owner，不表示完成。
    /// 既有预留和会话仍须由本原恢复责任实际排空，关闭无法撤销。
    pub fn seal_admission(&self) -> Result<(), EngineError> {
        self.registry.seal_admission()
    }

    /// 参数：host 为原 Engine 的扫描宿主；返回：是否共享原同一资源表，禁止错配恢复责任。
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    pub(crate) fn belongs_to(&self, host: &crate::ScanWorkerHost) -> bool {
        Arc::ptr_eq(&self.registry, &host.registry)
    }

    /// 参数：registry为Engine服务引用的原固定容量表；返回：外部唯一责任句柄。
    pub(super) fn new(registry: Arc<ScanWorkerRegistry>) -> Self {
        Self { registry }
    }
    /// 参数：无；返回：实际非空槽总数，不把Engine Drop或物理停止当已reaped。
    pub fn occupied_slots(&self) -> Result<usize, EngineError> {
        self.registry.occupied()
    }
    /// 参数：deadline 为宿主提供的同一绝对期限；返回：true 仅所有原槽实际回收。
    /// Linux/Windows/macOS 单次轮询；到期/竞争/Pending 返回 false，原错误返回 Err，均不丢 owner。
    /// 清理在锁外；归还 owner 的短状态锁和 OS 单调用不承诺硬墙钟上限。
    /// 未完成时调用者必须继续保留本 Recovery；这不是进程可以安全退出的声明。
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    pub fn drain_until(&self, deadline: std::time::Instant) -> Result<bool, EngineError> {
        self.registry.drain_until(deadline)
    }
    /// 参数：无；返回：true仅全部原wait闭环，false为仍在执行栈的槽，Err保原清理原因并保owner。
    /// cleanup/wait在registry和DB锁外。可能受内核I/O/调度影响，不承诺硬响应期限。
    /// 调用方须先停止并join实际runner；不得在Err后丢弃本句柄。
    pub fn drain(&self) -> Result<bool, EngineError> {
        self.registry.drain()
    }
}
