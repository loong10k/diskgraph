//! 测试服务沿用真实部署宿主及独立恢复责任，不替代 scanner 或后台 job。
use crate::{McpConfig, McpService, ScanWorkerSettings};
use diskgraph_engine::{EngineError, ScanWorkerRecovery, ScanWorkerRuntimeBudget};
use diskgraph_scan_worker::ProtocolLimits;
use std::ops::Deref;
use std::time::{Duration, Instant};

/// 保活隔离数据目录并持有同一服务的原扫描恢复责任。
/// 来源：原生 Rust PF-06 MCP 回归，无 Java 对等对象。
pub(crate) struct McpTestDirectory {
    recovery: Option<ScanWorkerRecovery>,
    directory: tempfile::TempDir,
}

impl McpTestDirectory {
    /// 参数：config、directory 保留原测试主体、工具集合与隔离数据目录。
    /// 返回：本地测试服务及外部恢复责任；宿主未配置时沿用旧明确拒绝扫描的服务。
    /// 不授予远程请求身份，不伪造扫描结果，不忽略失败。
    pub(crate) fn open(
        config: McpConfig,
        directory: tempfile::TempDir,
    ) -> Result<(McpService, Self), EngineError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let runtime = ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 1 << 20,
                max_stream_bytes: 2 << 30,
                max_nodes: 2_000_000,
                max_depth: 4096,
            },
            64 << 10,
            1,
        )?;
        let host = ScanWorkerSettings::host_from_environment(runtime, deadline, &mut || Ok(()))?;
        let (service, recovery) = match host {
            Some(host) => {
                let (service, recovery) = McpService::open_with_scan_worker(config, host)?;
                (service, Some(recovery))
            }
            None => (McpService::open(config)?, None),
        };
        Ok((
            service,
            Self {
                recovery,
                directory,
            },
        ))
    }
}

impl Deref for McpTestDirectory {
    type Target = tempfile::TempDir;
    fn deref(&self) -> &Self::Target {
        &self.directory
    }
}

impl Drop for McpTestDirectory {
    fn drop(&mut self) {
        if let Some(recovery) = &self.recovery {
            // runner 和调用线程必须先停止；同一 registry 完成前保活真实临时目录。
            // 沿用兼容等待路径，不把测试析构或外层超时计为有限退出证明。
            let mut reported = false;
            loop {
                match recovery.drain() {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) if !reported => {
                        eprintln!("test scan recovery incomplete; retaining original ownership");
                        reported = true;
                    }
                    Err(_) => {}
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
