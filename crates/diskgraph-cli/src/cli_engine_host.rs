use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerRecovery, ScanWorkerRuntimeBudget,
    ScanWorkerSettings,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// CLI 单次命令的可信宿主及独立进程恢复责任；来源：原生 Rust PF-06，无 Java 对等对象。
/// 普通命令与 init 使用同一入口；恢复句柄不进入命令的 catch_unwind。
pub(crate) struct CliEngineHost {
    engine: Arc<Engine>,
    recovery: Option<ScanWorkerRecovery>,
    #[cfg(windows)]
    probe_recovery: diskgraph_engine::ProbeRecovery,
}

impl CliEngineHost {
    /// 打开本地部署镜像后构造引擎，部分或非法配置不创建数据库。
    /// 参数：config 保留请求的节点、staging 与扫描选项；返回：引擎及独立恢复责任。
    pub(crate) fn open(config: EngineConfig) -> Result<Self, EngineError> {
        // 整次启动使用一个绝对期限；平台准入先于数据库，macOS只读取固定受保护安装。
        let deadline = Instant::now() + Duration::from_secs(30);
        let runtime = ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 1 << 20,
                max_stream_bytes: 2 << 30,
                max_nodes: config.max_nodes_per_scan,
                max_depth: 4096,
            },
            64 << 10,
            1,
        )?;
        let host = ScanWorkerSettings::host_from_environment(runtime, deadline, &mut || Ok(()))?;
        #[cfg(windows)]
        let (probe_host, probe_recovery) = diskgraph_engine::ProbeHost::new(1)?;
        #[cfg(windows)]
        let (engine, recovery) = Engine::open_with_process_hosts(config, host, probe_host)?;
        #[cfg(not(windows))]
        let (engine, recovery) = if let Some(host) = host {
            let (engine, recovery) = Engine::open_with_scan_worker(config, host)?;
            (engine, Some(recovery))
        } else {
            (Engine::open(config)?, None)
        };
        Ok(Self {
            engine: Arc::new(engine),
            recovery,
            #[cfg(windows)]
            probe_recovery,
        })
    }

    /// 执行单次命令；正常失败或 panic 后均先实际处置全部原进程。
    /// 参数：operation 只借用共享 Engine，不取得恢复句柄；返回：原业务结果或继续原 panic。
    pub(crate) fn execute<T>(
        self,
        operation: impl FnOnce(&Arc<Engine>) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&self.engine)));
        // CLI 没有后台扫描 runner；命令栈结束后才能清理该栈移交的原 child。
        if let Some(recovery) = &self.recovery {
            let mut reported = false;
            loop {
                // 先关闭同一资源池准入；失败仍保留原责任，不以空槽观察允许新工作出生。
                match recovery.seal_admission().and_then(|()| recovery.drain()) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) if !reported => {
                        eprintln!(
                            "scan worker recovery incomplete; shutdown retains ownership and waits"
                        );
                        reported = true;
                    }
                    Err(_) => {}
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        #[cfg(windows)]
        {
            let mut reported = false;
            loop {
                match self
                    .probe_recovery
                    .seal_admission()
                    .and_then(|()| self.probe_recovery.drain())
                {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) if !reported => {
                        eprintln!(
                            "probe recovery incomplete; shutdown retains ownership and waits"
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_engine;
#[cfg(test)]
pub(crate) use test_engine::CliTestEngine;
