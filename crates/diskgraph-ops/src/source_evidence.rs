//! source_evidence：既有文件操作职责的原生 Rust 实现。
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::bound_path;
use crate::ops_error::OpsError;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use sha2::Digest;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use sha2::Sha256;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::io::Read;
use std::path::Path;

/// 从元数据提取原有身份信息。
/// 参数：path 为原生路径；metadata 为对应元数据。
/// 返回：Unix 的设备和 inode 字符串；其他平台为 None。
/// The identity used to detect a replaced object between plan and apply: on
/// unix this is the device and inode, which survive a rename but not a
/// delete-and-recreate.
pub(super) fn identity_of(path: &Path, metadata: &std::fs::Metadata) -> Option<String> {
    // Both branches name the parameters they were handed. Only a unix
    // filesystem records a device and inode that survive a rename, and the
    // other platforms get None rather than a guess. A branch that referred
    // to a name it did not have would not compile there - and a mac-only
    // build never notices, which is how this sat broken for weeks.
    #[cfg(unix)]
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, metadata);
        None
    }
}

/// 一次源读取确认的身份、长度、摘要和已批准元数据。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::SourceEvidence`，保留既有语义。
/// Evidence bound into an approval. The digest covers bytes and nanosecond
/// metadata; the identity alone cannot detect an in-place rewrite.
pub(super) struct SourceEvidence {
    pub(super) identity: Option<String>,
    pub(super) fingerprint: String,
    pub(super) bytes: u64,
    pub(super) metadata: std::fs::Metadata,
}

/// 通过受约束源句柄读取并验证源证据。
/// 参数：path 为逐组件约束的源路径；max_bytes 为整文件读取上限。
/// 返回：已复核的 SourceEvidence 或不支持、变化、I/O 错误。
pub(super) fn capture_source(path: &Path, max_bytes: u64) -> Result<SourceEvidence, OpsError> {
    let _hydration = diskgraph_engine::content::HydrationGuard::enter()?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::fs::MetadataExt;
        let bound = bound_path::BoundPath::open(path)
            .map_err(|error| OpsError::Stale(format!("source path changed: {error}")))?;
        let mut file = bound
            .read()
            .map_err(|error| OpsError::Stale(format!("source path changed: {error}")))?;
        let before = file.metadata()?;
        if !before.is_file() {
            return Err(OpsError::Stale(
                "unsupported: this operation needs a regular file source".into(),
            ));
        }
        if before.len() > max_bytes {
            return Err(OpsError::Stale(format!(
                "source needs {} bytes, budget is {max_bytes}",
                before.len()
            )));
        }
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut chunk = [0_u8; 8192];
        loop {
            let read_limit = usize::try_from(max_bytes.saturating_sub(bytes).saturating_add(1))
                .unwrap_or(chunk.len())
                .min(chunk.len());
            let count = file.read(&mut chunk[..read_limit])?;
            if count == 0 {
                break;
            }
            bytes = bytes.saturating_add(count as u64);
            if bytes > max_bytes {
                return Err(OpsError::Stale(
                    "source exceeded the approved byte budget".into(),
                ));
            }
            digest.update(&chunk[..count]);
        }
        let after = file.metadata()?;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || bytes != before.len()
        {
            return Err(OpsError::Stale(
                "source changed while being evidenced".into(),
            ));
        }
        digest.update(before.len().to_le_bytes());
        digest.update(before.mtime().to_le_bytes());
        digest.update(before.mtime_nsec().to_le_bytes());
        digest.update(before.ctime().to_le_bytes());
        digest.update(before.ctime_nsec().to_le_bytes());
        Ok(SourceEvidence {
            identity: identity_of(path, &before),
            fingerprint: hex::encode(digest.finalize()),
            bytes,
            metadata: before,
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (path, max_bytes);
        Err(OpsError::Stale(
            "unsupported: this platform has no verified source handles".into(),
        ))
    }
}
