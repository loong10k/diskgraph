//! 本地宿主显式持有 accept 和全部连接线程，不改变业务 job 生命周期。
use crate::McpService;
use crate::http::ServerConfig;
use crate::http_shutdown_state::HttpShutdownState;
use std::io::Write;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::JoinHandle;

/// 受控 HTTP 服务 owner；来源：原生 Rust PF-06，无 Java 对等对象。
/// Drop 可等待在途业务完成，不能作为硬期限退出保证。
pub struct HttpServerRuntime {
    state: Arc<HttpShutdownState>,
    thread: Option<JoinHandle<std::io::Result<usize>>>,
}
impl HttpServerRuntime {
    /// 参数：原服务、监听器、安全配置及日志；返回：实际线程 owner 或原出生错误。
    pub fn start(
        service: McpService,
        listener: TcpListener,
        config: ServerConfig,
        log: impl Write + Send + 'static,
    ) -> std::io::Result<Self> {
        listener.set_nonblocking(true)?;
        let state = Arc::new(HttpShutdownState::default());
        let child_state = Arc::clone(&state);
        let thread = std::thread::Builder::new()
            .name("diskgraph-http".into())
            .spawn(move || {
                crate::http::serve_config_with_runtime(service, listener, config, log, child_state)
            })?;
        Ok(Self {
            state,
            thread: Some(thread),
        })
    }
    /// 参数：无；返回：仍登记的原连接数，仅诊断，不能替代实际 join。
    pub fn active_connections(&self) -> usize {
        self.state.active()
    }
    /// 参数：消费原服务 owner；返回：停止接入、关闭原连接并实际 join 后的原业务计数、IO错误或 panic。
    /// 不取消业务 job；在途业务仍使用原期限。
    pub fn stop_and_join(mut self) -> std::thread::Result<std::io::Result<usize>> {
        self.state.stop();
        self.thread.take().expect("unique HTTP thread owner").join()
    }
}
impl Drop for HttpServerRuntime {
    fn drop(&mut self) {
        self.state.stop();
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => eprintln!("HTTP shutdown failed: {error}"),
                Err(payload) if !std::thread::panicking() => std::panic::resume_unwind(payload),
                Err(_) => eprintln!("HTTP server panicked during foreground recovery"),
            }
        }
    }
}
