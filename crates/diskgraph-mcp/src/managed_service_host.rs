//! Unix MCP 的实际启动容量准入；不改变远程请求权限。
use diskgraph_engine::recovery_slot::{ActiveSlot, SlotReservation};
use diskgraph_engine::{
    EngineError, ScanWorkerHost, ScanWorkerRecovery, TrustedLocalRecoveryDomain,
};
use diskgraph_mcp::{McpConfig, McpService};
use std::time::Instant;

/// 协议出生前的原服务、扫描恢复责任与容量槽；不声明独立监督进程。
/// 来源：DiskGraph 原生 Rust PF-06 MCP 宿主准入合同，无 Java 对等对象。
pub(crate) struct ManagedServiceHost {
    pub(crate) service: McpService,
    pub(crate) recovery: Option<ScanWorkerRecovery>,
    pub(crate) slot: Option<ActiveSlot>,
}
impl ManagedServiceHost {
    /// 参数：config/host 为原本地部署，trusted_local 由传输决定，deadline 为原启动期限。
    /// 返回：原服务及其恢复材料，或数据库出生前的容量拒绝。
    pub(crate) fn open(
        config: McpConfig,
        host: Option<ScanWorkerHost>,
        trusted_local: bool,
        deadline: Instant,
    ) -> Result<Self, EngineError> {
        let managed = host.is_some();
        Self::open_admitted(config, host, trusted_local, deadline, || {
            if !managed {
                return Ok(None);
            }
            TrustedLocalRecoveryDomain::for_current_user(deadline)?
                .reserve(deadline)
                .map(Some)
                .map_err(EngineError::from)
        })
    }
    fn open_admitted(
        config: McpConfig,
        host: Option<ScanWorkerHost>,
        trusted_local: bool,
        deadline: Instant,
        reserve: impl FnOnce() -> Result<Option<SlotReservation>, EngineError>,
    ) -> Result<Self, EngineError> {
        if Instant::now() >= deadline {
            return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
        }
        let managed = host.is_some();
        let reservation = reserve()?;
        if managed != reservation.is_some() {
            if let Some(reservation) = reservation {
                reservation.abort_before_birth(deadline)?;
            }
            return Err(diskgraph_core::BusinessError::RecoveryUnconfirmed.into());
        }
        if Instant::now() >= deadline {
            return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
        }
        let opened = match (trusted_local, host) {
            (true, Some(host)) => {
                McpService::open_with_scan_worker(config, host).map(|(s, r)| (s, Some(r)))
            }
            (false, Some(host)) => {
                McpService::open_remote_with_scan_worker(config, host).map(|(s, r)| (s, Some(r)))
            }
            (true, None) => McpService::open(config).map(|s| (s, None)),
            (false, None) => McpService::open_remote(config).map(|s| (s, None)),
        };
        let opened = match opened {
            Ok(opened) => opened,
            Err(primary) => {
                if let Some(reservation) = reservation
                    && let Err(cleanup) = reservation.abort_before_birth(deadline)
                {
                    return Err(EngineError::WithCleanup {
                        primary: Box::new(primary),
                        cleanup: std::io::Error::other(cleanup),
                    });
                }
                return Err(primary);
            }
        };
        let slot = reservation.map(|r| r.activate(deadline)).transpose()?;
        // 无受管理宿主时也必须拒绝迟到的构造结果；此时尚无 runner 或原生工作。
        if !managed && Instant::now() >= deadline {
            return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
        }
        Ok(Self {
            service: opened.0,
            recovery: opened.1,
            slot,
        })
    }
}
#[cfg(test)]
mod tests;
