//! Unix 内容身份与路径预检。

use crate::EngineError;
use diskgraph_core::BusinessError;
use std::path::Path;

/// 记录 Unix 原生身份及观察版本。
/// 参数：metadata 为已观察的元数据。
/// 返回：卷/inode、长度及修改/change 时间指纹。
/// 身份指纹同时记录长度及高精度修改信息，原地写入也使结果失效。
#[cfg(unix)]
pub(crate) fn file_identity(metadata: &std::fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!(
            "{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

/// 重新观察路径版本并检查是否与原指纹相同。
/// 参数：before 为原指纹，path 为原路径。
/// 返回：是否仍观察到同一身份及版本。
#[cfg(unix)]
pub(crate) fn identity_stable(before: &Option<String>, path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .ok()
        .and_then(|after| file_identity(&after))
        .is_some_and(|after| Some(&after) == before.as_ref())
}

/// 拒绝目录、设备、FIFO 与 socket 内容对象。
/// 参数：path/metadata 为获取对象与元数据。
/// 返回：普通文件通过或非法参数。
/// Refuses everything that is not a plain file (CT-01): directories,
/// FIFOs, sockets, and devices are not content objects.
#[cfg(unix)]
pub(crate) fn ensure_plain_file(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), EngineError> {
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let kind = metadata.file_type();
        if kind.is_dir()
            || kind.is_fifo()
            || kind.is_socket()
            || kind.is_block_device()
            || kind.is_char_device()
        {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        if !metadata.is_file() {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
    }
    Ok(())
}

/// 保留 Unix 路径预检的范围与链接门禁。
/// 参数：root/path 为注册根与请求路径。
/// 返回：预检成功或定位/范围/链接拒绝；另须原生句柄核验。
/// Checks the object is inside the scope and not a link at any planned
/// component: the final component must be a real object, and the canonical
/// parent must stay under the canonical scope root (SC-03).
#[cfg(unix)]
pub(crate) fn ensure_inside_scope(root: &Path, path: &Path) -> Result<(), EngineError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| EngineError::Business(BusinessError::NotFound))?;
    if metadata.file_type().is_symlink() {
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    }
    let parent = path.parent().unwrap_or(Path::new("/"));
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| EngineError::Business(BusinessError::NotFound))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| EngineError::Business(BusinessError::NotFound))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(EngineError::Business(BusinessError::PermissionDenied));
    }
    Ok(())
}
