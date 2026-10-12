//! Unix MCP 的实际启动容量准入；不改变远程请求权限。
#[cfg(any(target_os = "linux", test))]
use diskgraph_engine::SupervisorOwner;
use diskgraph_engine::recovery_slot::{ActiveSlot, SlotReservation};
use diskgraph_engine::{
    EngineError, ScanWorkerHost, ScanWorkerRecovery, ScanWorkerRuntimeBudget,
    SupervisorControlOwner, TrustedLocalRecoveryDomain,
};
use diskgraph_mcp::{McpConfig, McpService};
use std::time::Instant;

/// 协议出生前的原服务、扫描恢复责任与容量槽；不声明独立监督进程。
/// 来源：DiskGraph 原生 Rust PF-06 MCP 宿主准入合同，无 Java 对等对象。
pub(crate) struct ManagedServiceHost {
    pub(crate) service: McpService,
    pub(crate) recovery: Option<ScanWorkerRecovery>,
    pub(crate) slot: Option<ActiveSlot>,
    pub(crate) control: Option<SupervisorControlOwner>,
    #[cfg(target_os = "linux")]
    pub(crate) namespace_guard: Option<diskgraph_engine::recovery_slot::LinuxSupervisorNamespace>,
}
impl ManagedServiceHost {
    /// 参数：config/runtime为固定宿主配置与扫描额度，trusted_local由传输决定，deadline为原启动期限。
    /// 返回：普通宿主或原认证监督宿主；出生值存在时禁止忽略或兼容回退，认证先于数据库。
    pub(crate) fn open_environment(
        config: McpConfig,
        runtime: ScanWorkerRuntimeBudget,
        trusted_local: bool,
        deadline: Instant,
    ) -> Result<Self, EngineError> {
        #[cfg(target_os = "linux")]
        if let Some(birth) =
            diskgraph_engine::LinuxExecutorBirth::receive_from_environment(deadline)?
        {
            if config.data_dir != birth.data_dir() {
                return Err(diskgraph_core::BusinessError::PermissionDenied.into());
            }
            let (host, writer, deadline) = birth.into_host(runtime, deadline)?;
            // 传输决定本地信任；出生材料不能授予远程管理员，也不移交 guardian 容量槽。
            let (service, recovery) =
                McpService::open_with_host_until(config, Some(host), trusted_local, deadline)?;
            return Self {
                service,
                recovery,
                slot: None,
                control: None,
                namespace_guard: None,
            }
            .attach_executor_control(writer, deadline);
        }
        #[cfg(target_os = "linux")]
        if let Some(birth) = diskgraph_engine::LinuxSupervisorBirth::receive_from_environment()? {
            if !birth.is_lifecycle() || config.data_dir != birth.data_dir() {
                return Err(diskgraph_core::BusinessError::PermissionDenied.into());
            }
            let admitted = birth.into_supervised_host(runtime, deadline)?;
            let control = admitted
                .control
                .ok_or(diskgraph_core::BusinessError::PermissionDenied)?;
            let mut opened = Self::open_admitted(
                config,
                Some(admitted.host),
                trusted_local,
                admitted.deadline,
                || {
                    admitted
                        .namespace
                        .reserve(admitted.deadline)
                        .map(Some)
                        .map_err(EngineError::from)
                },
            )?;
            opened.namespace_guard = Some(admitted.namespace);
            return opened.attach_control(control, admitted.deadline);
        }
        let host = diskgraph_mcp::ScanWorkerSettings::host_from_environment(
            runtime,
            deadline,
            &mut || Ok(()),
        )
        .map_err(|error| EngineError::Io(std::io::Error::other(error)))?;
        Self::open(config, host, trusted_local, deadline)
    }

    #[cfg(target_os = "linux")]
    fn attach_executor_control(
        mut self,
        writer: diskgraph_engine::recovery_control::UnixControlWriter,
        deadline: Instant,
    ) -> Result<Self, EngineError> {
        let parts = self
            .service
            .clone()
            .into_executor_parts(self.recovery.take());
        let owner = match diskgraph_engine::ExecutorResourceOwner::bind(parts) {
            Ok(owner) => owner,
            Err(parts) => {
                // 协议尚未启动；先释放全部服务引用，随后等待原扫描资源实际退休。
                drop(self.service);
                drop(parts.engine);
                crate::scan_worker_shutdown::finish(parts.scan.as_ref());
                return Err(diskgraph_core::BusinessError::RecoveryUnconfirmed.into());
            }
        };
        let mut owner = SupervisorControlOwner::for_executor(owner, writer);
        if owner
            .ready_lifecycle(&std::sync::atomic::AtomicBool::new(false), deadline)
            .is_err()
        {
            drop(self.service);
            crate::scan_worker_shutdown::finish_controlled(owner, 10, false);
            return Err(diskgraph_core::BusinessError::RecoveryUnconfirmed.into());
        }
        self.control = Some(owner);
        Ok(self)
    }

    #[cfg(any(target_os = "linux", test))]
    fn attach_control(
        mut self,
        writer: diskgraph_engine::recovery_control::UnixControlWriter,
        deadline: Instant,
    ) -> Result<Self, EngineError> {
        let parts = self.service.clone().into_supervisor_parts(
            self.recovery.take(),
            self.slot
                .take()
                .ok_or(diskgraph_core::BusinessError::RecoveryUnconfirmed)?,
        );
        let owner = match SupervisorOwner::bind(parts, deadline) {
            Ok(owner) => owner,
            Err(parts) => {
                // 未启动runner/协议；失败仍先释放同一服务引用并退休原资源，不丢ACTIVE。
                drop(self.service);
                crate::scan_worker_shutdown::finish_original(parts);
                return Err(diskgraph_core::BusinessError::RecoveryUnconfirmed.into());
            }
        };
        let mut owner = SupervisorControlOwner::new(owner, writer);
        if owner
            .ready_lifecycle(&std::sync::atomic::AtomicBool::new(false), deadline)
            .is_err()
        {
            drop(self.service);
            crate::scan_worker_shutdown::finish_controlled(owner, 10, false);
            return Err(diskgraph_core::BusinessError::RecoveryUnconfirmed.into());
        }
        self.control = Some(owner);
        Ok(self)
    }
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
        let opened = McpService::open_with_host_until(config, host, trusted_local, deadline);
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
            control: None,
            #[cfg(target_os = "linux")]
            namespace_guard: None,
        })
    }
}
#[cfg(test)]
mod tests;
