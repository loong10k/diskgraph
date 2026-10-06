use crate::api_result::ApiResult;
use crate::native_realm::{engine_config, local_principal};
use diskgraph_engine::{
    Engine, EngineError, ScanWorkerRecovery, ScanWorkerRuntimeBudget, ScanWorkerSettings,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// 旧同步扫描调用的栈上宿主与唯一物理恢复责任；来源：原生Rust PF-06，无Java对应。
/// 阻塞兼容入口仅供后台线程；不宣称有限时间退出，不把Recovery放入共享Engine。
pub(crate) struct NativeScanHost {
    engine: Arc<Engine>,
    recovery: Option<ScanWorkerRecovery>,
}

impl NativeScanHost {
    /// 参数：database为真实图库路径、cancel为原调用取消标志。
    /// 返回：统一平台材料准入后的引擎和外部恢复责任，或明确失败。
    pub(crate) fn open(database: &str, cancel: &AtomicBool) -> Result<Self, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let config = engine_config(database)?;
        let runtime = ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 1 << 20,
                max_stream_bytes: 2 << 30,
                max_nodes: config.max_nodes_per_scan,
                max_depth: 4096,
            },
            64 << 10,
            1,
        )
        .map_err(|error| error.to_string())?;
        let host = ScanWorkerSettings::host_from_environment(runtime, deadline, &mut || {
            if cancel.load(Ordering::SeqCst) {
                Err(EngineError::Io(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "cancelled during native scan host admission",
                )))
            } else {
                Ok(())
            }
        })
        .map_err(|error| error.to_string())?;
        let (engine, recovery) = match host {
            Some(host) => {
                let (engine, recovery) = Engine::open_with_scan_worker(config, host)
                    .map_err(|error| error.to_string())?;
                (engine, Some(recovery))
            }
            None => (
                Engine::open(config).map_err(|error| error.to_string())?,
                None,
            ),
        };
        engine
            .bootstrap_local_admin(&local_principal()?)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            engine: Arc::new(engine),
            recovery,
        })
    }

    /// 参数：operation为会结束并真实join协调runner的同步业务闭包。
    /// 返回：原业务值/错误；panic在实际清理后继续传播，不返回部分成功。
    /// 兼容入口会等待原恢复槽，不设伪期限；有限退场需后续宿主交接合同验收。
    pub(crate) fn execute(self, operation: impl FnOnce(Arc<Engine>) -> ApiResult) -> ApiResult {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            operation(Arc::clone(&self.engine))
        }));
        // 业务栈（包括NativeRunnerGuard的真实join）结束后才处理物理helper。
        if let Some(recovery) = &self.recovery {
            let mut reported = false;
            loop {
                match recovery.drain() {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) if !reported => {
                        eprintln!(
                            "FFI scan recovery incomplete; retains original ownership: {error}"
                        );
                        reported = true;
                    }
                    Err(_) => {}
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
