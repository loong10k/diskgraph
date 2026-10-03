//! path_codec：既有文件操作职责的原生 Rust 实现。
use std::path::Path;
use std::path::PathBuf;

/// 编码原生路径定位键。
/// 参数：path 为待记录路径。
/// 返回：平台对应的原生路径键。
/// A stable, reversible key for a path (the raw bytes, hex-encoded). It is the
/// identity used in plans and recovery records, never a display string.
pub(super) fn locator_key(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hex::encode(path.as_os_str().as_bytes())
    }
    #[cfg(not(unix))]
    {
        hex::encode(path.to_string_lossy().as_bytes())
    }
}

/// 沿现存祖先规范化目录路径，保留尚未存在的末段。
/// 参数：path 为待确认路径。
/// 返回：原有规范化或回退路径；此函数不认证目录或消除所有竞态。
/// The canonical form of a directory that may not exist yet: canonicalize the
/// deepest existing ancestor and re-attach the rest.
pub(super) fn canonical_dir(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut existing = path.to_path_buf();
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
    while let Some(parent) = existing.parent().map(Path::to_path_buf) {
        if let Some(name) = existing.file_name() {
            trailing.push(name.to_os_string());
        }
        if let Ok(canonical) = parent.canonicalize() {
            let mut result = canonical;
            for name in trailing.iter().rev() {
                result.push(name);
            }
            return result;
        }
        existing = parent;
    }
    path.to_path_buf()
}

/// 将定位器还原为原生路径。
/// 参数：locator 为资源定位器。
/// 返回：原生路径；定位键无法解码时为 None。
/// The native path behind a scope record's root locator.
pub(super) fn path_of(locator: &diskgraph_core::Locator) -> Option<PathBuf> {
    let bytes = locator.raw_bytes().ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
    }
}

/// 解码 Unix 路径十六进制键。
/// 参数：key 为定位键。
/// 返回：Unix 原生字节路径或非 Unix 既有展示路径；无效十六进制返回 None。
/// Reverses the hex locator key back into a path.
pub(super) fn unhex_key(key: &str) -> Option<PathBuf> {
    let bytes = hex::decode(key).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
    }
}
