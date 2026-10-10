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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    slot: Option<diskgraph_engine::recovery_slot::ActiveSlot>,
    #[cfg(target_os = "linux")]
    namespace_guard: Option<diskgraph_engine::recovery_slot::LinuxSupervisorNamespace>,
    #[cfg(windows)]
    probe_recovery: diskgraph_engine::ProbeRecovery,
}

impl CliEngineHost {
    /// 打开本地部署镜像后构造引擎，部分或非法配置不创建数据库。
    /// 参数：config 保留请求的节点、staging 与扫描选项；返回：引擎及独立恢复责任。
    pub(crate) fn open(config: EngineConfig) -> Result<Self, EngineError> {
        // 整次启动使用一个绝对期限；平台准入先于数据库，macOS只读取固定受保护安装。
        let deadline = Instant::now() + Duration::from_secs(30);
        Self::open_until(config, deadline)
    }

    /// 沿调用方原启动期限构造；参数：config为原配置，deadline不得在角色切换或排队后刷新。
    /// 返回：期限内host或原拒绝；不认证监督角色，不取得容量槽，不保证同步I/O硬抢占。
    pub(crate) fn open_until(config: EngineConfig, deadline: Instant) -> Result<Self, EngineError> {
        if Instant::now() >= deadline {
            return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
        }
        #[cfg(target_os = "linux")]
        if let Some(birth) = diskgraph_engine::LinuxSupervisorBirth::receive_from_environment()? {
            if config.data_dir != birth.data_dir() {
                return Err(diskgraph_core::BusinessError::PermissionDenied.into());
            }
            let runtime = Self::runtime_budget(&config)?;
            let (host, namespace, deadline) = birth.into_host(runtime, deadline)?;
            // 原 root/ns 材料在 Engine 初始化、出生前撤销及退休整个调用栈中均保持。
            let mut opened = Self::open_admitted_until(config, Some(host), deadline, || {
                namespace.reserve(deadline).map(Some).map_err(slot_error)
            })?;
            opened.namespace_guard = Some(namespace);
            return Ok(opened);
        }
        let host = Self::prepare_host_until(&config, deadline).inspect_err(|_| {
            // 仅报告固定阶段，不输出安装路径、配置或凭据，原错误分类保持不变。
            eprintln!("diskgraph: startup stage=worker_admission failed");
        })?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let managed = host.is_some();
            Self::open_admitted_until(config, host, deadline, || {
                if !managed {
                    return Ok(None);
                }
                let domain =
                    diskgraph_engine::TrustedLocalRecoveryDomain::for_current_user(deadline)
                        .inspect_err(|_| {
                            // 固定阶段区分目录准入与槽认领；不输出用户目录或修改原错误。
                            eprintln!("diskgraph: recovery stage=user_domain failed");
                        })
                        .map_err(slot_error)?;
                domain
                    .reserve(deadline)
                    .inspect_err(|_| {
                        eprintln!("diskgraph: recovery stage=slot_acquisition failed");
                    })
                    .map(Some)
                    .map_err(slot_error)
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            #[cfg(windows)]
            let (probe_host, probe_recovery) = diskgraph_engine::ProbeHost::new(1)?;
            #[cfg(windows)]
            let (engine, recovery) =
                Engine::open_with_process_hosts_until(config, host, probe_host, deadline)?;
            #[cfg(not(windows))]
            let (engine, recovery) = if let Some(host) = host {
                let (engine, recovery) =
                    Engine::open_with_scan_worker_until(config, host, deadline)?;
                (engine, Some(recovery))
            } else {
                (Engine::open_until(config, deadline)?, None)
            };
            let opened = Self {
                engine: Arc::new(engine),
                recovery,
                #[cfg(windows)]
                probe_recovery,
            };
            opened.finish_open_until(deadline)
        }
    }

    fn prepare_host_until(
        config: &EngineConfig,
        deadline: Instant,
    ) -> Result<Option<diskgraph_engine::ScanWorkerHost>, EngineError> {
        let runtime = Self::runtime_budget(config)?;
        ScanWorkerSettings::host_from_environment(runtime, deadline, &mut || Ok(()))
    }

    fn runtime_budget(config: &EngineConfig) -> Result<ScanWorkerRuntimeBudget, EngineError> {
        ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 1 << 20,
                max_stream_bytes: 2 << 30,
                max_nodes: config.max_nodes_per_scan,
                max_depth: 4096,
            },
            64 << 10,
            1,
        )
    }

    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    fn open_in_test_domain(
        config: EngineConfig,
        directory: std::fs::File,
    ) -> Result<Self, EngineError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let host = Self::prepare_host_until(&config, deadline).inspect_err(|_| {
            // 仅报告固定阶段，不输出安装路径、配置或凭据，原错误分类保持不变。
            eprintln!("diskgraph: startup stage=worker_admission failed");
        })?;
        let managed = host.is_some();
        Self::open_admitted_until(config, host, deadline, || {
            if !managed {
                return Ok(None);
            }
            diskgraph_engine::TrustedLocalRecoveryDomain::from_host(directory, deadline)
                .map_err(slot_error)?
                .reserve(deadline)
                .map(Some)
                .map_err(slot_error)
        })
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn open_admitted_until(
        config: EngineConfig,
        host: Option<diskgraph_engine::ScanWorkerHost>,
        deadline: Instant,
        reserve: impl FnOnce() -> Result<
            Option<diskgraph_engine::recovery_slot::SlotReservation>,
            EngineError,
        >,
    ) -> Result<Self, EngineError> {
        if Instant::now() >= deadline {
            return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
        }
        // 持久预留必须先于数据库出生；原生工作只能在 ACTIVE 及原绑定建立后执行。
        let reservation = reserve().inspect_err(|_| {
            eprintln!("diskgraph: startup stage=recovery_reservation failed");
        })?;
        if host.is_some() != reservation.is_some() {
            if let Some(reservation) = reservation {
                reservation
                    .abort_before_birth(deadline)
                    .map_err(slot_error)?;
            }
            return Err(diskgraph_core::BusinessError::RecoveryUnconfirmed.into());
        }
        // 准入可能等待到原期限之后；构造前复验，失败仍沿已有 reservation 撤销路径。
        let opened = if Instant::now() >= deadline {
            Err(diskgraph_core::BusinessError::BudgetExceeded.into())
        } else if let Some(host) = host {
            Engine::open_with_scan_worker_until(config, host, deadline)
                .map(|(engine, recovery)| (engine, Some(recovery)))
        } else {
            Engine::open_until(config, deadline).map(|engine| (engine, None))
        };
        let (engine, recovery) = match opened {
            Ok(opened) => opened,
            Err(primary) => {
                eprintln!("diskgraph: startup stage=engine_initialization failed");
                // 构造只初始化数据库，未向消费者交出 Engine、未启动 runner/原生工作。
                // 保留原错误；原期限内不能确认出生前取消时，记录保持未确认。
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
        let slot = reservation
            .map(|slot| slot.activate(deadline).map_err(slot_error))
            .transpose()
            .inspect_err(|_| {
                eprintln!("diskgraph: startup stage=recovery_activation failed");
            })?;
        Self {
            engine: Arc::new(engine),
            recovery,
            slot,
            #[cfg(target_os = "linux")]
            namespace_guard: None,
        }
        .finish_open_until(deadline)
    }

    fn finish_open_until(self, deadline: Instant) -> Result<Self, EngineError> {
        if Instant::now() >= deadline {
            // 不返回迟到执行能力；恢复仍沿原host处置，不能丢掉最后原Recovery。
            return self.execute(|_| Err(diskgraph_core::BusinessError::BudgetExceeded.into()));
        }
        Ok(self)
    }

    fn seal_admission(&self) -> Result<(), EngineError> {
        let scan = self
            .recovery
            .as_ref()
            .map_or(Ok(()), |r| r.seal_admission());
        // 两个池均先尝试关闭；即使扫描关闭失败，也不让探针准入继续开放。
        #[cfg(windows)]
        let probe = self.probe_recovery.seal_admission();
        scan?;
        #[cfg(windows)]
        probe?;
        Ok(())
    }

    /// 执行单次命令；正常失败或 panic 后均先实际处置全部原进程。
    /// 参数：operation 只借用共享 Engine，不取得恢复句柄；返回：原业务结果或继续原 panic。
    pub(crate) fn execute<T>(
        self,
        operation: impl FnOnce(&Arc<Engine>) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&self.engine)));
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if self.slot.is_some() {
            return self.retire_original_supervisor(outcome);
        }
        // CLI 没有后台扫描 runner；命令栈结束后才能清理该栈移交的原 child。
        let mut reported = false;
        loop {
            let round = crate::recovery_round::recovery_round(
                || self.seal_admission(),
                || {
                    self.recovery.as_ref().map_or(Ok(true), |r| {
                        // 每个原池独立获得一轮恢复窗口；Pending 保留原 owner，不能阻塞另一池。
                        r.drain_until(Instant::now() + Duration::from_millis(50))
                    })
                },
                || {
                    #[cfg(windows)]
                    {
                        self.probe_recovery
                            .drain_until(Instant::now() + Duration::from_millis(50))
                    }
                    #[cfg(not(windows))]
                    {
                        Ok(true)
                    }
                },
            );
            match round {
                Ok(true) => break,
                Ok(false) => {}
                Err(_) if !reported => {
                    eprintln!("native recovery incomplete; shutdown retains ownership and waits");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn retire_original_supervisor<T>(
        self,
        outcome: std::thread::Result<Result<T, EngineError>>,
    ) -> Result<T, EngineError> {
        #[cfg(target_os = "linux")]
        let _namespace_guard = self.namespace_guard;
        let mut parts = diskgraph_engine::SupervisorParts {
            engine: self.engine,
            scan: self.recovery,
            slot: self.slot.expect("original admitted slot"),
        };
        // 原绑定错误仍保留全部材料；不丢弃容量或用新 Recovery 代替。
        let mut owner = loop {
            match diskgraph_engine::SupervisorOwner::bind(
                parts,
                Instant::now() + Duration::from_millis(50),
            ) {
                Ok(owner) => break owner,
                Err(original) => parts = original,
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut reported = false;
        loop {
            match owner.poll_retirement(Instant::now() + Duration::from_millis(50)) {
                Ok(true) => break,
                Ok(false) => {}
                Err(_) if !reported => {
                    eprintln!("native recovery incomplete; original supervisor retains ownership");
                    reported = true;
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn slot_error(error: diskgraph_engine::recovery_slot::SlotError) -> EngineError {
    EngineError::from(error)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_engine;
#[cfg(test)]
pub(crate) use test_engine::CliTestEngine;
