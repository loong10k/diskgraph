use crate::EngineError;
use crate::native_child::{ChildInputMode, WindowsChild};
use crate::scan_worker_error_projection::ScanWorkerErrorProjection;
use crate::windows_scan_image_lease::WindowsScanImageLease;
use std::cell::RefCell;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

/// 持有原映像租约的 Windows 扫描启动器。来源：PF-06/原生 Rust，无 Java 对应。
/// 挂起出生后先核验原进程名称，再恢复；原 child 在失败和恢复期间保留全部租约。
pub(crate) struct WindowsScanLauncher {
    image: Arc<WindowsScanImageLease>,
}

impl WindowsScanLauncher {
    /// 参数：image 为独立摘要已核验的原租约；返回：单次启动材料，不接受外部程序路径。
    pub(crate) fn new(image: Arc<WindowsScanImageLease>) -> Self {
        Self { image }
    }

    /// 参数：owner 为 catch 外原槽，deadline/checkpoint 为原请求预算；返回：原启动或准入错误。
    /// 空环境和保留映像父目录避免使用 scope 目录作为 DLL 搜索工作目录。
    pub(crate) fn spawn_into(
        self,
        owner: &mut Option<WindowsChild>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let parent = self
            .image
            .native_path()
            .parent()
            .ok_or(diskgraph_core::BusinessError::InvalidArgument)?;
        let mut command = Command::new(self.image.native_path());
        command.env_clear().current_dir(parent);
        // 三类同步检查依次借同一请求；不重置期限，不建立独立权限或隐藏清理线程。
        let checkpoint = RefCell::new(checkpoint);
        WindowsChild::spawn_into_with_binding(
            &mut command,
            ChildInputMode::WorkerControl,
            owner,
            || {
                self.image
                    .validate(deadline, &mut **checkpoint.borrow_mut())
            },
            || {
                (checkpoint.borrow_mut())()?;
                if Instant::now() >= deadline {
                    return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
                }
                Ok(())
            },
            Some(Arc::clone(&self.image)),
            |process| {
                self.image
                    .verify_process_name(process, deadline, &mut **checkpoint.borrow_mut())
            },
        )
        .map_err(ScanWorkerErrorProjection::launch)
    }
}
