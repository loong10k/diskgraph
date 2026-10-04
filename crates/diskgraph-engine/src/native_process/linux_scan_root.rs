use super::linux_metadata::capture;
use super::linux_open::{open_at, unique_mount};
use super::linux_proc_root::LinuxProcRoot;
use crate::EngineError;
use diskgraph_core::{
    BusinessError, IndexedFileEpoch, ProcessEvidenceFailureCode as Failure, UnixFileObservation,
    UnixObservationGap,
};
use diskgraph_disktree::NodeV2;
use std::cell::RefCell;
use std::ffi::CString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

/// 扫描期间 held 注册根与 procfs 域；来源：Linux D42，强身份只补充本次扫描普通文件。
/// 不跨文件系统猜身份，也不把旧节点在任务执行现场升级为已捕获。
pub(crate) struct LinuxScanRoot {
    path: PathBuf,
    root: File,
    procfs: LinuxProcRoot,
}
impl LinuxScanRoot {
    /// 参数：真实注册原生根及扫描原有检查；返回：原生租约或固定平台缺口。
    pub(crate) fn open(
        path: &Path,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<Result<Self, Failure>, EngineError> {
        check()?;
        let result = checked_native(check, &|native_check| Self::open_native(path, native_check))?;
        check()?;
        Ok(result)
    }
    fn open_native(path: &Path, check: &dyn Fn() -> Result<(), Failure>) -> Result<Self, Failure> {
        if !path.is_absolute() {
            return Err(Failure::Unsupported);
        }
        let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| Failure::Unsupported)?;
        let root = open_at(
            libc::AT_FDCWD,
            &name,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )?;
        check()?;
        let procfs = LinuxProcRoot::open(check)?;
        Ok(Self {
            path: path.to_owned(),
            root,
            procfs,
        })
    }
    /// 参数：本次扫描节点与原扫描检查；返回：真实捕获或有限缺口，授权失败仍原样传播。
    pub(crate) fn observe(
        &self,
        node: &NodeV2,
        path: &Path,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<(Option<UnixFileObservation>, Option<UnixObservationGap>), EngineError> {
        check()?;
        let relative = path
            .strip_prefix(&self.path)
            .map_err(|_| BusinessError::Conflict)?;
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Ok((None, Some(UnixObservationGap::Unsupported)));
        }
        let name = CString::new(relative.as_os_str().as_bytes())
            .map_err(|_| BusinessError::Unsupported)?;
        let result = checked_native(check, &|native_check| {
            let file = open_at(
                self.root.as_raw_fd(),
                &name,
                libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0x08 | 0x04 | 0x02 | 0x01 | 0x20,
            )?;
            capture(&file, &self.procfs.boot, native_check)
        })?;
        check()?;
        match result {
            Ok(observation) => {
                let IndexedFileEpoch::LinuxHandle { device, inode, .. } = observation.epoch()
                else {
                    return Err(BusinessError::Unsupported.into());
                };
                if !node
                    .identity
                    .as_ref()
                    .is_some_and(|id| id.volume_id == device.to_string() && id.file_id == *inode)
                    || node.self_modified != Some(observation.modified().0)
                {
                    return Ok((None, Some(UnixObservationGap::TreeMismatch)));
                }
                Ok((Some(observation), None))
            }
            Err(error) => Ok((None, Some(gap(error)))),
        }
    }
    /// 参数：原扫描检查；返回：当前根路径仍指同一 held 根对象；本扫描补充不保存逐祖先绑定链。
    pub(crate) fn validate(
        &self,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        check()?;
        let name = CString::new(self.path.as_os_str().as_bytes())
            .map_err(|_| BusinessError::Unsupported)?;
        let reopened = open_at(
            libc::AT_FDCWD,
            &name,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )
        .map_err(|_| BusinessError::Conflict)?;
        let before = self.root.metadata()?;
        let after = reopened.metadata()?;
        check()?;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || unique_mount(&self.root).map_err(|_| BusinessError::Conflict)?
                != unique_mount(&reopened).map_err(|_| BusinessError::Conflict)?
        {
            return Err(BusinessError::Conflict.into());
        }
        Ok(())
    }
}
/// 参数：原生有限失败；返回：扫描旁表缺口，不伪装已经验证的身份。
pub(crate) fn gap(error: Failure) -> UnixObservationGap {
    match error {
        Failure::Unsupported => UnixObservationGap::Unsupported,
        Failure::PermissionDenied => UnixObservationGap::Denied,
        Failure::Conflict => UnixObservationGap::Changed,
        _ => UnixObservationGap::CaptureFailed,
    }
}

// 回调失败保留原 Engine 分类，不能降成可提交的缺口记录。
fn checked_native<T>(
    check: &dyn Fn() -> Result<(), EngineError>,
    work: &dyn Fn(&dyn Fn() -> Result<(), Failure>) -> Result<T, Failure>,
) -> Result<Result<T, Failure>, EngineError> {
    let original = RefCell::new(None);
    let result = work(&|| {
        if original.borrow().is_some() {
            return Err(Failure::Cancelled);
        }
        if let Err(error) = check() {
            *original.borrow_mut() = Some(error);
            return Err(Failure::Cancelled);
        }
        Ok(())
    });
    if let Some(error) = original.into_inner() {
        return Err(error);
    }
    Ok(result)
}
