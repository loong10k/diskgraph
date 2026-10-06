use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRecovery,
    ScanWorkerRuntimeBudget,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::fs::File;
use std::ops::Deref;

/// 显式提供真实镜像与恢复责任的 Linux 扫描夹具；来源：原生 Rust PF-06 公开宿主 API。
/// 预期值由受控构建部署提供，不从镜像正文或邻接清单自行建立信任。
pub(super) struct NativeScanEngine {
    engine: Engine,
    recovery: ScanWorkerRecovery,
}

impl NativeScanEngine {
    /// 参数：config 保留原测试的数据库和扫描预算；返回：真实引擎与原恢复责任。
    /// 缺少受信部署材料直接失败，不回退到进程内扫描或跳过原生测试。
    pub(super) fn open(config: EngineConfig) -> Result<Self, EngineError> {
        let image = std::env::var_os("DISKGRAPH_SCAN_WORKER_PATH")
            .expect("native scan fixture requires the deployed actual Cargo worker");
        let expected = std::env::var("DISKGRAPH_SCAN_WORKER_SHA256")
            .expect("native scan fixture requires the independent deployment digest");
        assert_eq!(
            expected.len(),
            64,
            "deployment digest must contain 32 bytes"
        );
        let mut digest = [0; 32];
        for (byte, pair) in digest.iter_mut().zip(expected.as_bytes().chunks_exact(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
                .expect("deployment digest must be hexadecimal");
        }
        let bytes = std::env::var("DISKGRAPH_SCAN_WORKER_BYTES")
            .expect("native scan fixture requires the independent deployment length")
            .parse()
            .expect("deployment length must be an unsigned integer");
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
        let host = ScanWorkerHost::new(
            File::open(image)?,
            ScanWorkerHostConfig::from_expected_image(digest, bytes)?,
            runtime,
        )?;
        let (engine, recovery) = Engine::open_with_scan_worker(config, host)?;
        Ok(Self { engine, recovery })
    }
}

impl Deref for NativeScanEngine {
    type Target = Engine;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

impl Drop for NativeScanEngine {
    fn drop(&mut self) {
        // 夹具在业务 panic 之外拥有原恢复句柄；不将缺名或 pending 当作完成。
        // 这里沿用实际阻塞回收，不宣称产品前端的有限退出已经实现。
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
