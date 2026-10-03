//! 请求局部的扫描测试同步；生产构建不包含此模块。
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, Sender, channel, sync_channel},
};
use std::time::Duration;

/// 只读观察真实进度后的测试回调；来源：DiskGraph 原生 Rust FFI 测试。
pub(crate) type NativeScanProgressHook = Arc<dyn Fn(&Value) + Send + Sync>;

/// 真实进度已绑定授权后的测试门；来源：DiskGraph 原生 Rust FFI 撤权回归，无 Java 对应对象。
/// 门只阻塞测试回调，不持有 JobState、Engine 控制库、图库或 NativeService 的锁。
pub(crate) struct NativeScanGate {
    queued: Receiver<Value>,
    release: Option<Sender<()>>,
}

impl NativeScanGate {
    /// 建立单次请求门；参数：无；返回：测试端 RAII owner 与真实进度后的回调。
    pub(crate) fn new() -> (Self, NativeScanProgressHook) {
        let (queued_sender, queued) = sync_channel(1);
        let (release, released) = channel();
        let released = Mutex::new(released);
        let notified = AtomicBool::new(false);
        let hook = Arc::new(move |progress: &Value| {
            if progress.get("job_id").and_then(Value::as_str).is_none()
                || notified.swap(true, Ordering::SeqCst)
            {
                return;
            }
            // 真实 progress 闭包已经完成授权绑定并释放 JobState 锁。
            if queued_sender.send(progress.clone()).is_ok() {
                // Drop 断开通道也会解除等待，避免测试断言失败后留下挂起 worker。
                let _ = released.lock().unwrap().recv();
            }
        });
        (
            Self {
                queued,
                release: Some(release),
            },
            hook,
        )
    }

    /// 等待真正持久提交后的进度通知；参数：无；返回：进度或清楚的夹具准备失败。
    /// 60 秒仅为测试准备 watchdog，不修改任何生产请求或作业期限。
    pub(crate) fn wait_queued(&self) -> Value {
        self.queued.recv_timeout(Duration::from_secs(60)).expect(
            "native scan fixture did not publish its durable queued progress within setup watchdog",
        )
    }

    /// 解除本测试请求的暂停；参数：无；返回：无，重复调用安全。
    pub(crate) fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for NativeScanGate {
    fn drop(&mut self) {
        self.release();
    }
}

#[test]
fn dropping_native_scan_gate_releases_the_paused_worker() {
    let (gate, hook) = NativeScanGate::new();
    let worker = std::thread::spawn(move || {
        hook(&serde_json::json!({"job_id":"isolated-fixture","state":"queued"}));
    });
    assert_eq!(gate.wait_queued()["job_id"], "isolated-fixture");
    drop(gate);
    worker.join().unwrap();
}
