//! cross_volume_copy：既有文件操作职责的原生 Rust 实现。
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::bound_path;
use crate::ops_error::OpsError;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::verified_source;
use std::path::Path;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::PathBuf;

/// 跨卷复制状态，固定源证据和独占暂存文件；未支持平台的零资源状态明确拒绝。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::CrossVolumeCopy`，保留既有语义。
/// A staged cross-volume transfer (OP-05). The staged file lives on the
/// target's own volume, so publishing is a same-volume rename: an interrupted
/// transfer leaves a staging directory behind and the destination untouched.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) struct CrossVolumeCopy {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) target_handle: bound_path::BoundPath,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) staged_handle: bound_path::BoundPath,
    pub(super) staging_dir: PathBuf,
    #[cfg(all(test, target_os = "macos"))]
    pub(super) staged: PathBuf,
    pub(super) target: PathBuf,
    pub(super) verified: std::sync::Mutex<Option<(std::fs::File, std::fs::Metadata)>>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(super) verified_source: std::sync::Mutex<Option<verified_source::VerifiedSource>>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl CrossVolumeCopy {
    /// 建立原有复制状态和受约束句柄。
    /// 参数：target 为最终文件路径；purpose 为暂存目录的操作前缀。
    /// 返回：暂存状态或明确平台不支持、路径及 I/O 错误。
    /// Opens a transfer that will publish `target` from a staging directory
    /// next to it. The directory name is unique per transfer, so concurrent
    /// transfers into one directory never remove each other's work.
    pub(crate) fn open(target: &Path, purpose: &str) -> Result<Self, OpsError> {
        let directory = target
            .parent()
            .ok_or_else(|| OpsError::Stale("target has no parent directory".into()))?;
        let staging_dir = directory.join(format!(
            ".dg-{purpose}-staging-{}",
            uuid::Uuid::new_v4().simple()
        ));
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let target_handle = bound_path::BoundPath::open(target)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let staged_handle = target_handle.staging(
            staging_dir
                .file_name()
                .ok_or_else(|| OpsError::Stale("staging has no name".into()))?,
        )?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err(OpsError::Stale(
            "unsupported: bound staging directory".into(),
        ));
        #[cfg(all(test, target_os = "macos"))]
        let name = target
            .file_name()
            .ok_or_else(|| OpsError::Stale("target has no file name".into()))?
            .to_owned();
        Ok(Self {
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            target_handle,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            staged_handle,
            #[cfg(all(test, target_os = "macos"))]
            staged: staging_dir.join(&name),
            staging_dir,
            target: target.to_path_buf(),
            verified: std::sync::Mutex::new(None),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            verified_source: std::sync::Mutex::new(None),
        })
    }

    /// 以禁止覆盖的原子能力发布已验证暂存。
    /// 参数：self 持有目标与已验证暂存。
    /// 返回：发布成功或冲突、变化、不支持错误。
    /// Publishes the verified copy with a same-volume rename. The destination
    /// is rechecked so a file that appeared during the transfer is never
    /// overwritten.
    pub(crate) fn publish(&self) -> Result<(), OpsError> {
        if std::fs::symlink_metadata(&self.target).is_ok() {
            return Err(OpsError::TargetExists);
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let verified = self
                .verified
                .lock()
                .map_err(|_| OpsError::Stale("transfer state poisoned".into()))?;
            let (_file, metadata) = verified
                .as_ref()
                .ok_or_else(|| OpsError::Stale("copy is not verified".into()))?;
            self.staged_handle
                .rename_verified_to(&self.target_handle, metadata)?;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err(OpsError::Stale("unsupported: bound publication".into()));
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(name) = self.staging_dir.file_name() {
            self.target_handle.remove_directory(name);
        }
        Ok(())
    }

    /// 显式清理本状态持有的独占暂存。
    /// 参数：self 为复制状态；未支持平台状态不持有资源。
    /// 返回：无返回值；保留既有尽力清理，未支持零资源实现为空。
    /// Best-effort cleanup after a failed transfer.
    pub(crate) fn discard(&self) {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.staged_handle.discard();
            if let Some(name) = self.staging_dir.file_name() {
                self.target_handle.remove_directory(name);
            }
        }
    }

    /// 检查演练暂存目录是否已清理。
    /// 参数：self 为复制状态。
    /// 返回：暂存目录不存在时为 true。
    /// True once no staging directory remains, so drills can prove a transfer
    /// left no bytes behind.
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn staging_dir_absent(&self) -> bool {
        !self.staging_dir.exists()
    }

    /// 取得演练暂存文件路径。
    /// 参数：self 为复制状态。
    /// 返回：暂存路径的借用。
    /// The staged path, for drills that must observe the transfer directly.
    #[cfg(all(test, target_os = "macos"))]
    pub(super) fn staged_path(&self) -> &Path {
        &self.staged
    }
}

/// 跨卷复制状态，固定源证据和独占暂存文件；未支持平台的零资源状态明确拒绝。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::CrossVolumeCopy`，保留既有语义。
/// Cross-volume publication has no verified Windows implementation yet.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) struct CrossVolumeCopy;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl CrossVolumeCopy {
    /// 建立原有复制状态和受约束句柄。
    /// 参数：target 为最终文件路径；purpose 为暂存目录的操作前缀。
    /// 返回：暂存状态或明确平台不支持、路径及 I/O 错误。
    pub(crate) fn open(_target: &Path, _purpose: &str) -> Result<Self, OpsError> {
        Err(OpsError::Stale(
            "unsupported: bound staging directory".into(),
        ))
    }

    /// 在独占暂存文件中完成复制验证。
    /// 参数：source 与预算参数指定受控源及复制限制。
    /// 返回：验证后的处理字节或明确不支持、变化和 I/O 错误。
    pub(crate) fn stage_and_verify(
        &self,
        _source: &Path,
        _expected_identity: &Option<String>,
    ) -> Result<u64, OpsError> {
        Err(OpsError::Stale(
            "unsupported: copy metadata fidelity has not been verified on this platform".into(),
        ))
    }

    /// 保持已批准元数据进行有界复制及保真验证。
    /// 参数：参数指定源、已批准元数据、读取预算与实时检查闭包。
    /// 返回：完整已验证暂存字节或前置条件、保真及平台错误。
    pub(crate) fn stage_and_verify_bounded(
        &self,
        source: &Path,
        expected: &Option<String>,
        _max_bytes: u64,
        _approved: Option<&std::fs::Metadata>,
        _check: &dyn Fn() -> Result<(), OpsError>,
    ) -> Result<u64, OpsError> {
        self.stage_and_verify(source, expected)
    }

    /// 以禁止覆盖的原子能力发布已验证暂存。
    /// 参数：self 持有目标与已验证暂存。
    /// 返回：发布成功或冲突、变化、不支持错误。
    pub(crate) fn publish(&self) -> Result<(), OpsError> {
        Err(OpsError::Stale("unsupported: bound publication".into()))
    }

    /// 显式清理本状态持有的独占暂存。
    /// 参数：self 为复制状态；未支持平台状态不持有资源。
    /// 返回：无返回值；保留既有尽力清理，未支持零资源实现为空。
    pub(crate) fn discard(&self) {}
}
