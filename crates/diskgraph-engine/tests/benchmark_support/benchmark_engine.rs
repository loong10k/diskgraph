use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerRecovery, ScanWorkerRuntimeBudget,
    ScanWorkerSettings,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 成本测量的真实扫描宿主和唯一恢复责任；来源：PF-06 原生Rust显式宿主合同。
pub(crate) struct BenchmarkEngine {
    /// 原引擎的共享引用；测量线程必须在本宿主结束前完成。
    pub(crate) engine: Arc<Engine>,
    recovery: ScanWorkerRecovery,
}

impl BenchmarkEngine {
    /// 参数：config为本侧独占数据库与原扫描预算；返回：实际宿主，缺部署直接拒绝。
    pub(crate) fn open(config: EngineConfig) -> Result<Self, EngineError> {
        let runtime = ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 1 << 20,
                max_stream_bytes: 2 << 30,
                max_nodes: config.max_nodes_per_scan,
                max_depth: 4096,
            },
            64 << 10,
            4,
        )?;
        let host = ScanWorkerSettings::host_from_environment(
            runtime,
            Instant::now() + Duration::from_secs(30),
            &mut || Ok(()),
        )?
        .ok_or(diskgraph_core::BusinessError::Unsupported)?;
        let (engine, recovery) = Engine::open_with_scan_worker(config, host)?;
        Ok(Self {
            engine: Arc::new(engine),
            recovery,
        })
    }
}

impl Drop for BenchmarkEngine {
    fn drop(&mut self) {
        // 失败/panic后的原责任仍由测量宿主持有；阻塞回收不冒充产品有限退出。
        let mut reported = false;
        loop {
            match self.recovery.drain() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) if !reported => {
                    eprintln!("benchmark retains scan recovery: {error}");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
