//! 原监听器共享停止状态；socket 登记与停止在同一短临界区排序。
use std::collections::HashMap;
use std::net::{Shutdown, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// 原连接句柄的停止见证；来源：原生 Rust PF-06，无 Java 对等对象。
#[derive(Default)]
pub(crate) struct HttpShutdownState {
    stopped: AtomicBool,
    next: AtomicU64,
    sockets: Mutex<HashMap<u64, TcpStream>>,
}
impl HttpShutdownState {
    /// 参数：无；返回：宿主是否已发出停止请求。
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    /// 参数：原 accepted socket；返回：同一停止锁内登记的标识，停止后返回 None。
    pub(crate) fn register(&self, stream: &TcpStream) -> std::io::Result<Option<u64>> {
        let held = stream.try_clone()?;
        let mut sockets = self
            .sockets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.stopped() {
            return Ok(None);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        sockets.insert(id, held);
        Ok(Some(id))
    }
    /// 参数：实际完成线程对应标识；返回：无，释放该线程原 socket 见证。
    pub(crate) fn release(&self, id: u64) {
        self.sockets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
    }
    /// 参数：无；返回：登记中的原 socket 数，仅用于诊断，不替代 join。
    pub(crate) fn active(&self) -> usize {
        self.sockets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }
    /// 参数：无；返回：无，停止准入并 shutdown 原 held sockets，不取消业务 job。
    pub(crate) fn stop(&self) {
        let sockets = self
            .sockets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.stopped.store(true, Ordering::Release);
        for stream in sockets.values() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}
