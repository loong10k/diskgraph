use crate::OpsError;
use std::path::Path;

/// 将父目录固定到句柄，逐组件拒绝链接，原子发布时禁止覆盖任何既有目标。
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn rename_no_replace(
    source: &Path,
    target: &Path,
    expected_identity: Option<&str>,
) -> Result<(), OpsError> {
    let bound_source = crate::bound_path::BoundPath::open(source)?;
    let before = bound_source.read()?.metadata()?;
    if expected_identity.is_none()
        || crate::identity_of(source, &before).as_deref() != expected_identity
    {
        return Err(OpsError::Stale(
            "source identity changed before publication".into(),
        ));
    }
    let target = crate::bound_path::BoundPath::open(target)?;
    bound_source.rename_verified_to(&target, &before)
}

/// 无法提供原子禁止覆盖的平台拒绝发布。
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn rename_no_replace(
    _source: &Path,
    _target: &Path,
    _expected_identity: Option<&str>,
) -> Result<(), OpsError> {
    Err(OpsError::Stale(
        "unsupported: atomic no-replace publication".into(),
    ))
}
