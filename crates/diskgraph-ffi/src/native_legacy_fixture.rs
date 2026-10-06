use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRecovery,
    ScanWorkerRuntimeBudget,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::fs::File;

/// FFI 旧库授权隔离夹具持有的唯一扫描恢复外槽；来源：PF-06 原生 Rust 显式宿主合同。
/// 只用于已有测试，不授予写工具权限，也不将阻塞回收称为产品有限退出。
pub(super) struct NativeLegacyFixture {
    recovery: ScanWorkerRecovery,
}

impl NativeLegacyFixture {
    /// 参数：config 为原夹具的数据库、容量和扫描节点预算。
    /// 返回：供旧库准备使用的原 Engine 与独立恢复责任；缺部署直接失败。
    pub(super) fn open(config: EngineConfig) -> Result<(Engine, Self), EngineError> {
        let path = std::env::var_os("DISKGRAPH_SCAN_WORKER_PATH")
            .expect("FFI legacy fixture requires actual worker deployment");
        let digest = std::env::var("DISKGRAPH_SCAN_WORKER_SHA256")
            .expect("FFI legacy fixture requires independent image digest");
        assert_eq!(digest.len(), 64, "deployment requires 32-byte digest");
        let mut expected = [0; 32];
        for (byte, pair) in expected.iter_mut().zip(digest.as_bytes().chunks_exact(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
                .expect("deployment digest must be hexadecimal");
        }
        let bytes = std::env::var("DISKGRAPH_SCAN_WORKER_BYTES")
            .expect("FFI legacy fixture requires independent image length")
            .parse()
            .expect("deployment length must be unsigned");
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
            File::open(path)?,
            ScanWorkerHostConfig::from_expected_image(expected, bytes)?,
            runtime,
        )?;
        let (engine, recovery) = Engine::open_with_scan_worker(config, host)?;
        Ok((engine, Self { recovery }))
    }
}

impl Drop for NativeLegacyFixture {
    fn drop(&mut self) {
        // 业务 panic/错误仍保留同一恢复责任，不释放未确认的原槽。
        let mut reported = false;
        loop {
            match self.recovery.drain() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) if !reported => {
                    eprintln!("FFI legacy fixture retains original scan recovery: {error}");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
