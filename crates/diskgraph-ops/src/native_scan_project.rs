use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerRecovery, ScanWorkerRuntimeBudget,
    ScanWorkerSettings,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::sync::Arc;

/// Ops 隔离夹具持有的唯一扫描恢复外槽；来源：PF-06 原生 Rust 显式宿主合同。
/// 只用于已有测试，不授予写工具权限，也不将阻塞回收称为产品有限退出。
pub(super) struct NativeScanProject {
    recovery: ScanWorkerRecovery,
}

impl NativeScanProject {
    /// 参数：config 为原夹具的数据库、容量和扫描节点预算。
    /// 返回：提供给真实 PlanBuilder 的原 Arc<Engine> 与独立恢复责任；缺部署直接失败。
    pub(super) fn open(config: EngineConfig) -> Result<(Arc<Engine>, Self), EngineError> {
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
            std::time::Instant::now() + std::time::Duration::from_secs(30),
            &mut || Ok(()),
        )?
        .ok_or(diskgraph_core::BusinessError::Unsupported)?;
        let (engine, recovery) = Engine::open_with_scan_worker(config, host)?;
        Ok((Arc::new(engine), Self { recovery }))
    }
}

impl Drop for NativeScanProject {
    fn drop(&mut self) {
        // 业务 panic/错误仍保留同一恢复责任，不释放未确认的原槽。
        let mut reported = false;
        loop {
            match self.recovery.drain() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) if !reported => {
                    eprintln!("Ops fixture retains original scan recovery: {error}");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
