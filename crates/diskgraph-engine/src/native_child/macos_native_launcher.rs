use super::child_read_buffer::ChildReadBuffer;
use super::macos_native_pipes::MacosNativePipes;
use super::unix_child_setup::UnixChildSetup;
use super::{ChildError, UnixChild};
use crate::EngineError;
use crate::macos_installation_lease::MacosInstallationLease;
use crate::scan_worker_error_projection::ScanWorkerErrorProjection;
use diskgraph_core::BusinessError;
use std::ffi::CString;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::sync::Arc;
use std::time::Instant;

/// 仅消费已认证安装租约的公开Darwin启动器；固定argv/空环境，无fork或宿主pre_exec。
/// 来源：原生 Rust PF-06 macOS native spawn 合同；无 Java 对等对象。
/// 路径exec仍依赖可信发行方active namespace合同；本类型不证明fresh安装或NPROC出生限制。
pub(crate) struct MacosNativeLauncher {
    lease: Arc<MacosInstallationLease>,
    program: CString,
    pipes: MacosNativePipes,
    buffer: ChildReadBuffer,
    update_lock: Option<crate::macos_installation_lock::MacosInstallationLock>,
}

impl MacosNativeLauncher {
    /// 出生前准备全部资源。参数：lease是独立信任准入材料，期限/检查点沿原请求；返回：预备启动器或原准备错误。
    #[cfg(test)]
    pub(super) fn prepare(
        lease: Arc<MacosInstallationLease>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        Self::prepare_inner(lease, None, deadline, checkpoint)
    }

    fn prepare_inner(
        lease: Arc<MacosInstallationLease>,
        update_lock: Option<crate::macos_installation_lock::MacosInstallationLock>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        check(deadline, checkpoint)?;
        let program = CString::new(lease.native_path().as_bytes()).map_err(|_| {
            ScanWorkerErrorProjection::child(ChildError::Unsupported(
                "native installation path contains NUL",
            ))
        })?;
        let buffer = ChildReadBuffer::new().map_err(ScanWorkerErrorProjection::child)?;
        let pipes = MacosNativePipes::prepare(super::ChildInputMode::WorkerControl, &mut || {
            check(deadline, checkpoint)
        })?;
        Ok(Self {
            lease,
            program,
            pipes,
            buffer,
            update_lock,
        })
    }

    /// 消费Host锁内核验的单次出生材料，保留更新锁跨通道准备与原C出生。
    /// 参数：permit是唯一守卫，期限/检查点沿原请求；返回：启动器或原错误。
    pub(crate) fn prepare_qualified(
        permit: crate::macos_spawn_permit::MacosSpawnPermit,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let (installation, update_lock) = permit.into_parts();
        Self::prepare_inner(installation, Some(update_lock), deadline, checkpoint)
    }

    /// 将原child直接出生到caller外槽。参数：owner/deadline/checkpoint沿原claim；返回：原错误。
    /// 出生后的任何错误或panic都保留外槽；caller必须dispose/retain，不能丢弃owner。
    pub(crate) fn spawn_into(
        self,
        owner: &mut Option<UnixChild>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        self.spawn_inner(owner, deadline, checkpoint, |_| {})
    }
    /// 参数：owner 为外部原槽，deadline/checkpoint 为原期限，hook 仅观察实际 PID；返回：出生结果或原错误，失败责任仍留原槽。
    ///
    /// 测试专用出生观察点；参数：hook只收到原PID，不能在生产入口注入任意回调。
    #[cfg(test)]
    pub(super) fn spawn_observed(
        self,
        owner: &mut Option<UnixChild>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
        hook: impl FnOnce(u32),
    ) -> Result<(), EngineError> {
        self.spawn_inner(owner, deadline, checkpoint, hook)
    }

    fn spawn_inner(
        self,
        owner: &mut Option<UnixChild>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
        hook: impl FnOnce(u32),
    ) -> Result<(), EngineError> {
        if owner.is_some() {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, checkpoint)?;
        UnixChildSetup::reject_auto_reap().map_err(ScanWorkerErrorProjection::child)?;
        self.lease.revalidate(deadline, checkpoint)?;
        check(deadline, checkpoint)?;
        let gate = super::native_birth_gate::NativeBirthGate::acquire(&mut || {
            check(deadline, checkpoint)
        })?;
        let MacosNativePipes {
            control,
            input,
            stdout,
            output,
            stderr,
            diagnostic,
        } = self.pipes;
        let channels = [
            input.as_raw_fd(),
            output.as_raw_fd(),
            diagnostic.as_raw_fd(),
        ];
        // 最早原owner在C调用前即含原管道、缓冲和lease；C直接填原PID与group责任位。
        *owner = Some(UnixChild::prepare_native(
            self.buffer,
            control,
            stdout,
            stderr,
            self.lease,
        ));
        let status = owner
            .as_mut()
            .expect("prebirth external owner is present")
            .birth_native(&self.program, channels);
        drop((input, output, diagnostic));
        drop(gate);
        // 原PID/group责任和出生返回值已保存；此后才允许可信更新者发布新epoch。
        drop(self.update_lock);
        if status != 0 {
            // C非零保证没有child；只释放未生资源，原返回errno不被Drop诊断覆盖。
            owner.take();
            return Err(std::io::Error::from_raw_os_error(status).into());
        }
        let pid = owner
            .as_mut()
            .expect("born owner remains in original caller slot")
            .initialize_native()
            .map_err(ScanWorkerErrorProjection::child)?;
        hook(pid);
        check(deadline, checkpoint)
    }
}

fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
