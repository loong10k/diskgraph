//! 仅测试线程的清理展开注入，生产构建不包含本模块。
use std::cell::Cell;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// 对原恢复流程注入一次 panic，不改变身份或模拟清理成功。来源：DiskGraph 原生 Rust，无 Java 对象。
pub(crate) struct ProbePoolCleanupFault;

impl ProbePoolCleanupFault {
    /// 标记本线程下一次清理展开；无参数，无返回。
    pub(crate) fn arm() {
        ARMED.set(true);
    }

    /// 消费本线程的一次故障并保留固定原 panic；无参数，无正常返回值。
    pub(crate) fn checkpoint() {
        if ARMED.replace(false) {
            std::panic::panic_any("original-probe-pool-cleanup-panic");
        }
    }
}
