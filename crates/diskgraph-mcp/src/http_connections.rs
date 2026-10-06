//! accept 线程持有的连接线程集合；等待发生在共享状态锁外。
use crate::http_shutdown_state::HttpShutdownState;
use std::sync::Arc;
use std::thread::JoinHandle;

/// 有界连接线程 owner；来源：原生 Rust PF-06，无 Java 对等对象。
pub(crate) struct HttpConnections {
    state: Arc<HttpShutdownState>,
    threads: Vec<JoinHandle<()>>,
    failure: Option<Box<dyn std::any::Any + Send>>,
}
impl HttpConnections {
    /// 参数：同一宿主停止状态；返回：唯一连接线程集合。
    pub(crate) fn new(state: Arc<HttpShutdownState>) -> Self {
        Self {
            state,
            threads: Vec::new(),
            failure: None,
        }
    }
    /// 参数：实际出生线程句柄；返回：无，接管其 join 责任。
    pub(crate) fn push(&mut self, thread: JoinHandle<()>) {
        self.threads.push(thread);
    }
    /// 参数：无；返回：无，锁外 join 已结束线程，保留首个原 panic。
    pub(crate) fn reap(&mut self) {
        let mut i = 0;
        while i < self.threads.len() {
            if self.threads[i].is_finished() {
                let thread = self.threads.swap_remove(i);
                self.join(thread);
            } else {
                i += 1;
            }
        }
    }
    /// 参数：无；返回：尚未 join 的线程数，包含已完成但未回收线程。
    pub(crate) fn len(&self) -> usize {
        self.threads.len()
    }
    fn join(&mut self, thread: JoinHandle<()>) {
        if let Err(payload) = thread.join() {
            if self.failure.is_none() {
                self.failure = Some(payload);
            }
            self.state.stop();
        }
    }
    /// 参数：无；返回：全部原线程已 join，若有后台 panic 则恢复原 payload。
    pub(crate) fn finish(&mut self) {
        self.state.stop();
        while let Some(thread) = self.threads.pop() {
            self.join(thread);
        }
        if let Some(payload) = self.failure.take() {
            if !std::thread::panicking() {
                std::panic::resume_unwind(payload);
            }
            eprintln!("HTTP connection panicked during foreground recovery");
        }
    }
}
impl Drop for HttpConnections {
    fn drop(&mut self) {
        self.finish();
    }
}
