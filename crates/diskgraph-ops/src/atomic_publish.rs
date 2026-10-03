use crate::OpsError;
use std::path::Path;

/// 将批准时捕获的文件版本绑定到禁止覆盖发布，不重新采集预期版本。
#[cfg(any(target_os = "macos", target_os = "linux"))]
/// 参数：source 和 target 为源、目标路径；approved 为批准时固定的元数据。
/// 返回：禁止覆盖且版本匹配时 Ok；否则为冲突、变化或平台不支持错误。
pub(crate) fn rename_approved_no_replace(
    source: &Path,
    target: &Path,
    approved: &std::fs::Metadata,
) -> Result<(), OpsError> {
    let source = crate::bound_path::BoundPath::open(source)?;
    let target = crate::bound_path::BoundPath::open(target)?;
    source.rename_verified_to(&target, approved)
}

/// 无可靠句柄语义时拒绝批准版本发布。
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
/// 参数：source 和 target 为源、目标路径；approved 为批准时固定的元数据。
/// 返回：禁止覆盖且版本匹配时 Ok；否则为冲突、变化或平台不支持错误。
pub(crate) fn rename_approved_no_replace(
    _source: &Path,
    _target: &Path,
    _approved: &std::fs::Metadata,
) -> Result<(), OpsError> {
    Err(OpsError::Stale(
        "unsupported: approved version publication".into(),
    ))
}
