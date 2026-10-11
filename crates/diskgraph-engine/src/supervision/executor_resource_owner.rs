use crate::{Engine, EngineError, ExecutorParts, ScanWorkerRecovery};
use diskgraph_core::BusinessError;
use std::sync::Arc;
use std::time::Instant;

/// 原executor资源的本进程退休责任；来源：PF-06 guardian/executor合同，无Java对等对象。
/// 不持容量槽或CLEAN能力，资源完成不证明原executor进程退出或跨进程交付受信。
#[must_use = "retain original executor owner until resource retirement is confirmed"]
pub struct ExecutorResourceOwner {
    engine: Option<Arc<Engine>>,
    scan: Option<ScanWorkerRecovery>,
    #[cfg(windows)]
    probe: Option<crate::ProbeRecovery>,
    retired: bool,
}

impl ExecutorResourceOwner {
    /// 参数：parts为原Engine及全部恢复责任；返回：同一绑定或整体原材料，不封闭错配池。
    pub fn bind(parts: ExecutorParts) -> Result<Self, ExecutorParts> {
        let scan_matches = match (&parts.engine.scan_worker, &parts.scan) {
            (Some(host), Some(recovery)) => recovery.belongs_to(host),
            (None, None) => true,
            _ => false,
        };
        #[cfg(windows)]
        let probe_matches = match (&parts.engine.probe_host, &parts.probe) {
            (Some(host), Some(recovery)) => recovery.belongs_to(host),
            (None, None) => true,
            _ => false,
        };
        #[cfg(not(windows))]
        let probe_matches = true;
        let managed = parts.engine.scan_worker.is_some();
        #[cfg(windows)]
        let managed = managed || parts.engine.probe_host.is_some();
        if !scan_matches || !probe_matches || !managed {
            return Err(parts);
        }
        Ok(Self {
            engine: Some(parts.engine),
            scan: parts.scan,
            #[cfg(windows)]
            probe: parts.probe,
            retired: false,
        })
    }

    /// 参数：无；返回：尚未开始销毁的原Engine引用，不重建或外传资源owner。
    pub fn engine(&self) -> Option<&Arc<Engine>> {
        self.engine.as_ref()
    }

    /// 参数：deadline为原单次恢复期限；返回：原资源实际完成才为true，错误或Pending保留原责任。
    /// guardian仍须验证原进程退休；本方法不读写容量槽，不授予CLEAN或远程授权。
    pub fn poll_retirement(&mut self, deadline: Instant) -> Result<bool, EngineError> {
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        if self.retired {
            return Ok(true);
        }
        // 两个原池均先尝试封闭；扫描失败也不能留下探针池继续出生新工作。
        let scan_sealed = self
            .scan
            .as_ref()
            .map_or(Ok(()), |scan| scan.seal_admission());
        #[cfg(windows)]
        let probe_sealed = self
            .probe
            .as_ref()
            .map_or(Ok(()), |probe| probe.seal_admission());
        scan_sealed?;
        #[cfg(windows)]
        probe_sealed?;
        if let Some(engine) = self.engine.take() {
            match Arc::try_unwrap(engine) {
                Ok(engine) => drop(engine),
                Err(engine) => {
                    self.engine = Some(engine);
                    return Ok(false);
                }
            }
        }
        // 两池在同一期限内各自推进；保留第一次错误，仍让另一池执行恢复。
        let scan_done = self
            .scan
            .as_ref()
            .map_or(Ok(true), |scan| scan.drain_until(deadline));
        #[cfg(windows)]
        let probe_done = self
            .probe
            .as_ref()
            .map_or(Ok(true), |probe| probe.drain_until(deadline));
        let scan_done = scan_done?;
        #[cfg(windows)]
        let probe_done = probe_done?;
        #[cfg(not(windows))]
        let probe_done = true;
        if !scan_done || !probe_done {
            return Ok(false);
        }
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        self.retired = true;
        Ok(true)
    }
}
