//! volume_identity：既有文件操作职责的原生 Rust 实现。
use crate::ops_error::OpsError;
use std::path::Path;

/// 比较源目标所在卷。
/// 参数：source 与 target 为待比较路径。
/// 返回：平台卷身份是否相同或元数据错误。
/// Same-volume operations use an atomic rename; the cross-volume paths are
/// handled by the dedicated P6 staging flow.
pub(super) fn are_same_volume(source: &Path, target: &Path) -> Result<bool, OpsError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let left = std::fs::metadata(source)
            .map_err(|error| OpsError::Stale(error.to_string()))?
            .dev();
        let right = std::fs::metadata(target.parent().unwrap_or(target))
            .map_err(|error| OpsError::Stale(error.to_string()))?
            .dev();
        Ok(left == right)
    }
    #[cfg(not(unix))]
    {
        // Without device ids the platform cannot prove same-volume, so every
        // transfer takes the staged path: correct, just slower.
        let _ = (source, target);
        Ok(false)
    }
}
