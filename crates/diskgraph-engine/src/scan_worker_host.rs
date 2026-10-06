use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{EngineError, ScanWorkerHostConfig, ScanWorkerRuntimeBudget};
use diskgraph_core::BusinessError;
use std::fs::File;
use std::sync::Arc;
#[cfg(not(windows))]
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::time::Duration;
use std::time::Instant;

/// 普通Rust可信宿主提供的held镜像、独立预期和原执行额度，不接受远程文件选择。
/// 来源：原生 Rust PF-06；配置只是材料，实际运行仍须sealed image/原子出生/正常wait。
pub struct ScanWorkerHost {
    #[cfg(target_os = "linux")]
    image: Mutex<File>,
    #[cfg(windows)]
    windows_image: Arc<crate::windows_scan_image_lease::WindowsScanImageLease>,
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    _image: Mutex<File>,
    #[cfg(target_os = "macos")]
    _image: Option<Mutex<File>>,
    #[cfg(target_os = "macos")]
    installation: Option<Arc<crate::macos_installation_lease::MacosInstallationLease>>,
    #[cfg(target_os = "macos")]
    active_settings: Option<crate::macos_host_settings::MacosHostSettings>,
    #[cfg(target_os = "linux")]
    expected: ScanWorkerHostConfig,
    pub(super) budget: ScanWorkerRuntimeBudget,
    pub(super) registry: Arc<ScanWorkerRegistry>,
}

impl ScanWorkerHost {
    /// 参数：held_image为已打开镜像，expected来自独立宿主信任，runtime为显式原额度/容量。
    /// 返回：唯一宿主材料；不核邻接清单，不启动进程，不自行取得请求权限。
    pub fn new(
        held_image: File,
        expected: ScanWorkerHostConfig,
        runtime: ScanWorkerRuntimeBudget,
    ) -> Result<Self, EngineError> {
        Self::new_until(
            held_image,
            expected,
            runtime,
            Instant::now() + std::time::Duration::from_secs(30),
            &mut || Ok(()),
        )
    }

    /// 参数：原宿主镜像/独立预期/额度及同一次deadline/checkpoint；返回：核验材料或原错误。
    /// 保留new可信本地签名；有启动预算的调用方必须传原绝对期限，Windows租约不授予执行许可。
    pub fn new_until(
        held_image: File,
        expected: ScanWorkerHostConfig,
        runtime: ScanWorkerRuntimeBudget,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let metadata = held_image.metadata()?;
        if !metadata.is_file() {
            return Err(BusinessError::Unsupported.into());
        }
        if metadata.len() != expected.expected_bytes {
            return Err(BusinessError::Conflict.into());
        }
        #[cfg(windows)]
        let held_image = crate::windows_scan_image_lease::WindowsScanImageLease::prepare(
            held_image, &expected, deadline, checkpoint,
        )?;
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let registry = ScanWorkerRegistry::new(runtime.max_active_children())?;
        Ok(Self {
            #[cfg(target_os = "linux")]
            image: Mutex::new(held_image),
            #[cfg(windows)]
            windows_image: Arc::new(held_image),
            #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
            _image: Mutex::new(held_image),
            #[cfg(target_os = "macos")]
            _image: Some(Mutex::new(held_image)),
            #[cfg(target_os = "macos")]
            installation: None,
            #[cfg(target_os = "macos")]
            active_settings: None,
            #[cfg(target_os = "linux")]
            expected,
            budget: runtime,
            registry,
        })
    }

    /// 核验借用进程报告的镜像名称与本宿主原材料绑定，不授予执行或恢复线程权限。
    /// 参数：process为调用方保留的真实进程句柄，deadline/checkpoint为原请求期限与检查。
    /// 返回：名称及原句柄版本一致，或原错误；不证明映射字节及既有可写映射已安全。
    #[cfg(windows)]
    pub fn verify_windows_process_image_name(
        &self,
        process: std::os::windows::io::BorrowedHandle<'_>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        self.windows_image
            .verify_process_name(process, deadline, checkpoint)
    }

    /// 借用不可变准入映像并复查原期限与身份。参数：原请求预算；返回：跨子进程恢复保留的租约。
    #[cfg(windows)]
    pub(super) fn prepare_windows_image(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Arc<crate::windows_scan_image_lease::WindowsScanImageLease>, EngineError> {
        self.windows_image.validate(deadline, checkpoint)?;
        Ok(Arc::clone(&self.windows_image))
    }

    /// 从产品固定root保护配置构造macOS宿主，普通环境和旧File入口不能提供此资格。
    /// 参数：runtime为显式额度，deadline/checkpoint为原构造期限；返回：宿主或未配置状态。
    /// 存在但非法的配置必须失败；本入口不自行安装、签发、更新epoch或授予请求权限。
    #[cfg(target_os = "macos")]
    pub fn from_installed_macos(
        runtime: ScanWorkerRuntimeBudget,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Option<Self>, EngineError> {
        let Some(_initial_settings) =
            crate::macos_host_settings::MacosHostSettings::read_active(deadline, checkpoint)?
        else {
            crate::macos_epoch_floor::MacosEpochFloor::ensure_unconfigured(deadline, checkpoint)?;
            return Ok(None);
        };
        let _update_lock = crate::macos_installation_lock::MacosInstallationLock::acquire_shared(
            deadline, checkpoint,
        )?;
        let settings = crate::macos_host_settings::MacosHostSettings::read_authorized(
            &_update_lock,
            deadline,
            checkpoint,
        )?;
        let receipt = crate::macos_protected_document::MacosProtectedDocument::read(
            &settings.receipt_path,
            16 * 1024,
            deadline,
            checkpoint,
        )?;
        let lease = crate::macos_installation_lease::MacosInstallationLease::admit(
            &receipt,
            &settings.trust()?,
            &settings.expected()?,
            deadline,
            checkpoint,
        )?;
        lease.validate_loader(deadline, checkpoint)?;
        let registry = ScanWorkerRegistry::new(runtime.max_active_children())?;
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Some(Self {
            _image: None,
            installation: Some(Arc::new(lease)),
            active_settings: Some(settings),
            budget: runtime,
            registry,
        }))
    }

    /// 在原请求检查点确认运行代仍为活跃安装，返回：跨发布事务保留的原共享锁。
    /// 参数：原期限/检查点；返回：守卫或原撤销/冲突错误，不在数据库锁内等待安装锁。
    #[cfg(target_os = "macos")]
    pub(super) fn authorize_macos_epoch(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<crate::macos_installation_lock::MacosInstallationLock, EngineError> {
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let original = self
            .active_settings
            .as_ref()
            .ok_or(BusinessError::Unsupported)?;
        let guard = crate::macos_installation_lock::MacosInstallationLock::acquire_shared(
            deadline, checkpoint,
        )?;
        let current = crate::macos_host_settings::MacosHostSettings::read_authorized(
            &guard, deadline, checkpoint,
        )?;
        if &current != original {
            return Err(BusinessError::Conflict.into());
        }
        Ok(guard)
    }

    /// 出生前重新读取固定活跃配置并核对原epoch/密钥/摘要和完整安装租约。
    /// 参数：期限/检查点沿原claim；返回：携带原lease及更新锁的单次材料，更新后旧Host明确拒绝。
    #[cfg(target_os = "macos")]
    pub(super) fn prepare_macos_installation(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<crate::macos_spawn_permit::MacosSpawnPermit, EngineError> {
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let installation = self
            .installation
            .as_ref()
            .ok_or(BusinessError::Unsupported)?;
        let original = self
            .active_settings
            .as_ref()
            .ok_or(BusinessError::Unsupported)?;
        let update_lock = crate::macos_installation_lock::MacosInstallationLock::acquire_shared(
            deadline, checkpoint,
        )?;
        let current = crate::macos_host_settings::MacosHostSettings::read_authorized(
            &update_lock,
            deadline,
            checkpoint,
        )?;
        if &current != original {
            return Err(BusinessError::Conflict.into());
        }
        installation.validate_loader(deadline, checkpoint)?;
        // 长耗时摘要核验之后再次读取，不能仅靠准备开始时的active epoch。
        let current = crate::macos_host_settings::MacosHostSettings::read_authorized(
            &update_lock,
            deadline,
            checkpoint,
        )?;
        if &current != original {
            return Err(BusinessError::Conflict.into());
        }
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(crate::macos_spawn_permit::MacosSpawnPermit::new(
            Arc::clone(installation),
            update_lock,
        ))
    }

    /// 参数：deadline/checkpoint沿原claim；返回：同次完整核验和真实四seal的唯一File。
    /// 串行保护共享open-file-description的seek/read，nativeIO不持DB/registry锁。
    #[cfg(target_os = "linux")]
    pub(super) fn prepare_image(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<File, EngineError> {
        let image = loop {
            checkpoint()?;
            if Instant::now() >= deadline {
                return Err(BusinessError::BudgetExceeded.into());
            }
            match self.image.try_lock() {
                Ok(image) => break image,
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(EngineError::Poisoned),
                Err(std::sync::TryLockError::WouldBlock) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    std::thread::sleep(remaining.min(Duration::from_millis(20)));
                }
            }
        };
        let sealed = crate::native_child::LinuxScanImage::prepare(
            image.try_clone()?,
            &self.expected,
            deadline,
            checkpoint,
        )?;
        Ok(sealed.into_file())
    }
}
