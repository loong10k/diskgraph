use crate::EngineError;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_child::ScanWorkerChild;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_driver::ScanWorkerDriver;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_error_projection::ScanWorkerErrorProjection;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_host::ScanWorkerHost;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_owned_failure::ScanWorkerOwnedFailure;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::scan_worker_reservation::ScanWorkerReservation;
use diskgraph_core::BusinessError;
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{DecodedTree, ScanProgress};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use diskgraph_scan_worker::{ExecutionOutcome, WorkerRequest};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::io::Write;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::thread;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::time::Duration;
use std::time::Instant;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

/// 同一claim期限中的唯一helper父运行栈，外层保留driver、panic槽和出生前容量预留。
/// 来源：原生 Rust PF-06；协议结果必须经过实际normal wait后才交回Engine转换。
pub(super) struct ScanWorkerRuntime<'a> {
    host: &'a ScanWorkerHost,
    deadline: Instant,
}

impl<'a> ScanWorkerRuntime<'a> {
    /// 参数：host为普通Rust受信宿主材料，deadline为原claim截止；返回：借用运行上下文。
    pub(super) fn new(host: &'a ScanWorkerHost, deadline: Instant) -> Self {
        Self { host, deadline }
    }

    /// 参数：root是原绝对scope根、options保全部原字段，检查点借原权限/fence/时钟。
    /// before_tick保原20ms循环/5sheartbeat，progress消费真实完整快照。
    /// 返回：已完整协议和原OSwait的迭代树owner，或原失败；panic原payload恢复前保留必要child。
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    pub(super) fn run(
        &self,
        root: &Path,
        options: &ScanOptions,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
        before_tick: &mut impl FnMut() -> Result<(), EngineError>,
        progress: &mut impl FnMut(&ScanProgress) -> Result<(), EngineError>,
    ) -> Result<DecodedTree, EngineError> {
        let reservation = self.host.registry.reserve()?;
        // 原launcher panic槽、成功出生owner与driver都在catch外，观察器unwind不会丢原child。
        let mut unwind_owner = None;
        let mut launched: Option<ScanWorkerChild> = None;
        let mut driver: Option<ScanWorkerDriver> = None;
        // 原授权检查先执行；macOS每次协议检查重新确认安装代，变化进入同一dispose/Recovery路径。
        #[cfg(target_os = "macos")]
        let mut epoch_checkpoint = || {
            self.host
                .authorize_macos_epoch(self.deadline, &mut *checkpoint)
                .map(|_| ())
        };
        #[cfg(target_os = "macos")]
        let checkpoint = &mut epoch_checkpoint;
        let observed = catch_unwind(AssertUnwindSafe(|| {
            checkpoint()?;
            self.check_clock()?;
            let request = WorkerRequest::scan(root, options, self.host.budget.response_limits())?;
            launched = Some(self.launch(checkpoint, &reservation, &mut unwind_owner)?);
            #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
            crate::scan_worker_runtime_hooks::after_launch(
                launched
                    .as_mut()
                    .expect("original launched child is owned outside catch"),
            );
            match ScanWorkerDriver::new(
                &mut launched,
                request,
                (env!("DISKGRAPH_ENGINE_TARGET"), PIN),
                self.deadline,
                self.host.budget.stderr_bytes(),
            ) {
                Ok(configured) => driver = Some(configured),
                Err(error) => {
                    // 配置失败时原 child 仍在 catch 外；实际处置后才投影一次原错。
                    let failure = ScanWorkerOwnedFailure::dispose(
                        error,
                        launched
                            .take()
                            .expect("configuration retained original child"),
                    );
                    let (error, owner) = failure.into_parts();
                    if let Some(owner) = owner {
                        reservation.retain(owner);
                    }
                    return Err(ScanWorkerErrorProjection::driver(
                        error,
                        |never| match never {},
                    ));
                }
            }
            loop {
                before_tick()?;
                let active = driver
                    .as_mut()
                    .expect("configured driver remains outside catch");
                let outcome = active
                    .poll(false, &mut *checkpoint)
                    .map_err(|error| ScanWorkerErrorProjection::driver(error, |primary| primary))?;
                if let Some(snapshot) = active.progress() {
                    progress(snapshot)?;
                }
                if let Some(outcome) = outcome {
                    return match outcome {
                        ExecutionOutcome::Tree(tree) => {
                            // Driver的许可已消费whole-group wait；再保留真实成功退出码事实。
                            if active.exit_code() != Some(0) {
                                return Err(std::io::Error::other(
                                    "scan worker tree lacks successful exit",
                                )
                                .into());
                            }
                            Ok(tree)
                        }
                        ExecutionOutcome::Failure(failure) => {
                            Err(ScanWorkerErrorProjection::remote(failure))
                        }
                    };
                }
                let remaining = self.deadline.saturating_duration_since(Instant::now());
                thread::sleep(remaining.min(Duration::from_millis(20)));
            }
        }));
        match observed {
            Ok(Ok(tree)) => {
                // 仅normal许可成功能交付；清理不是normal许可，未使用Drop推论已完成。
                drop(driver);
                drop(reservation);
                Ok(tree)
            }
            Ok(Err(error)) => {
                let error = if let Some(active) = driver.take() {
                    let failure = active.into_owned_failure(ScanWorkerFailure::Checkpoint {
                        primary: error,
                        cleanup: None,
                    });
                    let (error, owner) = failure.into_parts();
                    if let Some(owner) = owner {
                        reservation.retain(owner);
                    }
                    ScanWorkerErrorProjection::driver(error, |primary| primary)
                } else {
                    error
                };
                let error = if let Some(owner) = launched.take() {
                    let failure = ScanWorkerOwnedFailure::dispose(
                        ScanWorkerFailure::Checkpoint {
                            primary: error,
                            cleanup: None,
                        },
                        owner,
                    );
                    let (error, owner) = failure.into_parts();
                    if let Some(owner) = owner {
                        reservation.retain(owner);
                    }
                    ScanWorkerErrorProjection::driver(error, |primary| primary)
                } else {
                    error
                };
                if let Some(owner) = unwind_owner.take() {
                    reservation.retain(owner);
                }
                Err(error)
            }
            Err(payload) => {
                if let Some(owner) = launched.take() {
                    let (error, owner) = ScanWorkerOwnedFailure::dispose(
                        ScanWorkerFailure::<std::convert::Infallible>::Stopped { cleanup: None },
                        owner,
                    )
                    .into_parts();
                    if let Some(owner) = owner {
                        reservation.retain(owner);
                    }
                    if let ScanWorkerFailure::Stopped {
                        cleanup: Some(error),
                    } = error
                    {
                        // 原panic仍继续；独立真实清理诊断只写固定native上下文，不替换payload。
                        let _ = writeln!(
                            std::io::stderr(),
                            "scan worker unwind cleanup failed: {error}"
                        );
                    }
                }
                if let Some(active) = driver.take() {
                    let cleanup = active.unwind_cleanup_error().cloned();
                    let failed =
                        active.into_owned_failure(
                            ScanWorkerFailure::<std::convert::Infallible>::Stopped { cleanup },
                        );
                    let (error, owner) = failed.into_parts();
                    // 仅固定类型的后验说明；不替换panic payload，不打印helper/路径/任意错误消息。
                    let retained = owner.is_some();
                    if let Some(owner) = owner {
                        reservation.retain(owner);
                    }
                    let cleanup_failed = match error {
                        ScanWorkerFailure::Stopped { cleanup } => cleanup.is_some(),
                        _ => false,
                    };
                    let _ = writeln!(
                        std::io::stderr(),
                        "scan worker unwind: retained_owner={retained}, cleanup_failed={cleanup_failed}"
                    );
                }
                if let Some(owner) = unwind_owner.take() {
                    reservation.retain(owner);
                }
                // reservation Drop只释放无Retained的原槽；外部Recovery在resume前已拥有恢复责任。
                drop(reservation);
                resume_unwind(payload)
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    fn launch(
        &self,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
        reservation: &ScanWorkerReservation,
        unwind_owner: &mut Option<ScanWorkerChild>,
    ) -> Result<ScanWorkerChild, EngineError> {
        checkpoint()?;
        self.check_clock()?;
        #[cfg(target_os = "linux")]
        {
            use crate::native_child::LinuxAtomicLauncher;
            use std::ffi::CString;
            let image = self.host.prepare_image(self.deadline, checkpoint)?;
            let launcher = LinuxAtomicLauncher::prepare(
                image,
                vec![CString::new("diskgraph-scan-worker").expect("fixed name has no NUL")],
                Vec::new(),
            )
            .map_err(ScanWorkerErrorProjection::child)?;
            match launcher.spawn(self.deadline, checkpoint, unwind_owner) {
                Ok(child) => Ok(child),
                Err(failure) => {
                    let (error, owner) = failure.into_parts();
                    if let Some(owner) = owner {
                        reservation.retain(owner);
                    }
                    Err(ScanWorkerErrorProjection::launch(error))
                }
            }
        }
        #[cfg(all(target_os = "macos", feature = "macos_native_scan_candidate"))]
        {
            let _ = reservation;
            let permit = self
                .host
                .prepare_macos_installation(self.deadline, checkpoint)?;
            let launcher = crate::native_child::MacosNativeLauncher::prepare_qualified(
                permit,
                self.deadline,
                checkpoint,
            )?;
            // spawn直接填catch外原槽，错误/panic继续交由原retained owner和Recovery负责。
            launcher.spawn_into(unwind_owner, self.deadline, checkpoint)?;
            unwind_owner
                .take()
                .ok_or_else(|| BusinessError::Conflict.into())
        }
        #[cfg(any(
            all(target_os = "macos", not(feature = "macos_native_scan_candidate")),
            not(any(target_os = "linux", target_os = "macos"))
        ))]
        {
            let _ = (reservation, unwind_owner);
            // 没有已认证heldimage lease的macOS/Windows不得按路径启动或用Hello自授信任。
            Err(BusinessError::Unsupported.into())
        }
    }

    /// 参数：检查点沿原请求；返回：无已认证桌面执行器的平台明确Unsupported。
    /// 保留移动查询/管理构建；不以伪child或假normal wait提供扫描成功。
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    pub(super) fn run(
        &self,
        _root: &Path,
        _options: &ScanOptions,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
        _before_tick: &mut impl FnMut() -> Result<(), EngineError>,
        _progress: &mut impl FnMut(&ScanProgress) -> Result<(), EngineError>,
    ) -> Result<DecodedTree, EngineError> {
        checkpoint()?;
        self.check_clock()?;
        let _ = self.host;
        Err(BusinessError::Unsupported.into())
    }

    fn check_clock(&self) -> Result<(), EngineError> {
        if Instant::now() >= self.deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(())
    }
}
