use diskgraph_engine::{EngineError, ScanWorkerRecovery};
#[cfg(target_os = "linux")]
use diskgraph_engine::{ScanWorkerHost, ScanWorkerRuntimeBudget};
#[cfg(target_os = "linux")]
use diskgraph_mcp::ScanWorkerSettings;
use diskgraph_mcp::{McpConfig, McpService};

/// 测试外槽持有原扫描恢复责任；来源：原生 Rust PF-06 MCP 宿主合同。
/// 不复制 owner，不将 pending 或清理错误当作完成，不证明产品有限退出。
pub(crate) struct NativeScanRecovery {
    recovery: Option<ScanWorkerRecovery>,
}

/// 参数：config 保留原配置，remote 保留原本地/远程认证模式。
/// 返回：真实服务及必须在夹具整个生命周期保留的唯一恢复外槽。
/// Linux 要求受控部署预期；其他平台使用原构造，不伪造平台执行能力。
pub(crate) fn open(
    config: McpConfig,
    remote: bool,
) -> Result<(McpService, NativeScanRecovery), EngineError> {
    #[cfg(target_os = "linux")]
    {
        let settings = ScanWorkerSettings::from_environment()?
            .expect("native MCP fixture requires independent actual worker deployment");
        let (image, expected) = settings.open_held()?;
        let runtime = ScanWorkerRuntimeBudget::new(
            diskgraph_scan_worker::ProtocolLimits {
                max_frame_bytes: 1 << 20,
                max_stream_bytes: 2 << 30,
                max_nodes: 2_000_000,
                max_depth: 4096,
            },
            64 << 10,
            4,
        )?;
        let host = ScanWorkerHost::new(image, expected, runtime)?;
        let (service, recovery) = if remote {
            McpService::open_remote_with_scan_worker(config, host)?
        } else {
            McpService::open_with_scan_worker(config, host)?
        };
        Ok((
            service,
            NativeScanRecovery {
                recovery: Some(recovery),
            },
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let service = if remote {
            McpService::open_remote(config)?
        } else {
            McpService::open(config)?
        };
        Ok((service, NativeScanRecovery { recovery: None }))
    }
}

impl Drop for NativeScanRecovery {
    fn drop(&mut self) {
        let Some(recovery) = self.recovery.as_mut() else {
            return;
        };
        // 原错误只报告一次，持续保留责任；仅夹具沿用阻塞清理，不作为生产恢复门禁。
        let mut reported = false;
        loop {
            match recovery.drain() {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) if !reported => {
                    eprintln!("native MCP fixture retains original recovery: {error}");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
