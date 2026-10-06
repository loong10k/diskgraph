use super::{
    Engine, EngineConfig, EngineError, ScanWorkerRecovery, ScanWorkerRuntimeBudget,
    ScanWorkerSettings,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// 显式提供真实镜像与恢复责任的 三桌面扫描夹具；来源：原生 Rust PF-06 公开宿主 API。
/// 预期值由受控构建部署提供，不从镜像正文或邻接清单自行建立信任。
pub(crate) struct NativeScanEngine {
    /// 原Engine的共享引用；借出引用/线程必须先于夹具结束，恢复责任仍唯一持有。
    pub(crate) engine: Arc<Engine>,
    recovery: ScanWorkerRecovery,
    #[cfg(windows)]
    probe_recovery: super::ProbeRecovery,
}

impl NativeScanEngine {
    /// 参数：config 保留原测试的数据库和扫描预算；返回：真实引擎与原恢复责任。
    /// 缺少受信部署材料直接失败，不回退到进程内扫描或跳过原生测试。
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
            std::time::Instant::now() + std::time::Duration::from_secs(30),
            &mut || Ok(()),
        )?
        .ok_or(diskgraph_core::BusinessError::Unsupported)?;
        #[cfg(windows)]
        let (probe, probe_recovery) = super::ProbeHost::new(4)?;
        #[cfg(windows)]
        let (engine, recovery) = Engine::open_with_process_hosts(config, Some(host), probe)?;
        #[cfg(windows)]
        let recovery = recovery.expect("explicit admitted worker has original recovery");
        #[cfg(not(windows))]
        let (engine, recovery) = Engine::open_with_scan_worker(config, host)?;
        Ok(Self {
            engine: Arc::new(engine),
            recovery,
            #[cfg(windows)]
            probe_recovery,
        })
    }
}

impl Deref for NativeScanEngine {
    type Target = Engine;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

impl DerefMut for NativeScanEngine {
    /// 参数：夹具独占可变借用；返回：原Engine的可变借用，不移动或复制Recovery。
    /// 保留既有单元测试设置原扫描时限的语义，不开放任何生产权限入口。
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::get_mut(&mut self.engine).expect("exclusive test Engine required for mutation")
    }
}

impl Drop for NativeScanEngine {
    fn drop(&mut self) {
        // 夹具在业务 panic 之外拥有原恢复句柄；不将缺名或 pending 当作完成。
        // 这里沿用实际阻塞回收，不宣称产品前端的有限退出已经实现。
        #[cfg(windows)]
        loop {
            match self.probe_recovery.drain() {
                Ok(true) => break,
                Ok(false) | Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        let mut reported = false;
        loop {
            match self.recovery.drain() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) if !reported => {
                    eprintln!("native scan fixture retains recovery responsibility: {error}");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
