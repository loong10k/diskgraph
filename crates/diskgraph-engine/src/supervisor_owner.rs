use crate::recovery_slot::ActiveSlot;
use crate::{Engine, ScanWorkerRecovery, SupervisorParts, SupervisorRecoveryError};
use diskgraph_core::BusinessError;
use std::sync::Arc;
use std::time::Instant;
/// 原 Engine、恢复责任和 ACTIVE 容量的监督持有者；来源：PF-06，无 Java 对等对象。
/// 必须在前台 catch_unwind 外保留；Drop 不表示回收，不建立进程或 IPC 执行信任。
#[must_use = "retain the original owner until explicit retirement is confirmed"]
pub struct SupervisorOwner {
    engine: Option<Arc<Engine>>,
    scan: Option<ScanWorkerRecovery>,
    #[cfg(windows)]
    probe: Option<crate::ProbeRecovery>,
    slot: Option<ActiveSlot>,
}
impl SupervisorOwner {
    /// 参数：parts 为全部原材料，deadline 为原绑定期限；返回：同一原绑定或原材料整体拒绝，不丢弃恢复责任。
    pub fn bind(mut parts: SupervisorParts, deadline: Instant) -> Result<Self, SupervisorParts> {
        if parts.slot.verify_active(deadline).is_err() {
            return Err(parts);
        }
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
        let managed = parts.engine.scan_worker.is_some();
        #[cfg(windows)]
        let managed = managed || parts.engine.probe_host.is_some();
        #[cfg(not(windows))]
        let probe_matches = true;
        // 拒绝前不移动字段，不 seal，不写槽；原材料整体归还调用方。
        if !scan_matches || !probe_matches || !managed {
            return Err(parts);
        }
        Ok(Self {
            engine: Some(parts.engine),
            scan: parts.scan,
            #[cfg(windows)]
            probe: parts.probe,
            slot: Some(parts.slot),
        })
    }
    /// 参数：无；返回：尚未开始销毁的原 Engine 引用；退休后不再提供执行入口。
    pub fn engine(&self) -> Option<&Arc<Engine>> {
        self.engine.as_ref()
    }

    /// 参数：deadline 为原单次恢复绝对期限；返回：真实原资源确认及原槽释放才为 true。
    /// Pending/错误均保留原 owner；此方法不能代替真实前台结束、runner join 和进程退出验收。
    pub fn poll_retirement(&mut self, deadline: Instant) -> Result<bool, SupervisorRecoveryError> {
        if Instant::now() >= deadline {
            return Err(crate::EngineError::from(BusinessError::BudgetExceeded).into());
        }
        if self.slot.is_none() {
            return Ok(true);
        }
        if let Some(scan) = &self.scan {
            scan.seal_admission()?;
        }
        #[cfg(windows)]
        if let Some(probe) = &self.probe {
            probe.seal_admission()?;
        }
        if let Some(engine) = self.engine.take() {
            // 原子取得唯一 Engine；失败还回原 Arc，不能依据瞬时计数或弱引用推定独占。
            match Arc::try_unwrap(engine) {
                Ok(engine) => drop(engine),
                Err(engine) => {
                    self.engine = Some(engine);
                    return Ok(false);
                }
            }
        }
        if let Some(scan) = &self.scan
            && !scan.drain_until(deadline)?
        {
            return Ok(false);
        }
        #[cfg(windows)]
        if let Some(probe) = &self.probe
            && !probe.drain_until(deadline)?
        {
            return Ok(false);
        }
        if Instant::now() >= deadline {
            return Err(crate::EngineError::from(BusinessError::BudgetExceeded).into());
        }
        self.slot
            .as_mut()
            .expect("original active slot retained")
            .confirm_original_cleanup(deadline)?;
        // 只有同一锁内同步回读 CLEAN 成功后关闭锁；失败路径不消费 ACTIVE 能力。
        drop(self.slot.take());
        Ok(true)
    }
}
