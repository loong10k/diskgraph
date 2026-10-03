//! volume_capacity：既有文件操作职责的原生 Rust 实现。
use crate::ops_error::OpsError;
use crate::path_codec::unhex_key;
use diskgraph_core::ScopeId;
use diskgraph_engine::Engine;
use std::path::Path;

/// 测量路径所在卷的可用空间。
/// 参数：path 为卷上的路径。
/// 返回：macOS/Linux 的可用字节 Some；读取失败、路径不可表示或未支持平台返回 None。
/// Measures free space on the volume holding `path`, without shelling out to
/// `df` (EC-01). Returns None where the platform cannot answer.
pub fn volume_free_bytes(path: &Path) -> Option<u64> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        // Find the nearest existing ancestor, since a destination may not exist
        // yet, and read the filesystem's own block accounting.
        let mut probe = path.to_path_buf();
        loop {
            match std::fs::symlink_metadata(&probe) {
                Ok(_) => break,
                Err(_) => probe = probe.parent()?.to_path_buf(),
            }
        }
        let c_path = std::ffi::CString::new(probe.to_str()?).ok()?;
        // SAFETY: c_path is a valid NUL-terminated string and stat is a valid
        // pointer to a fully initialised statfs. libc owns the layout, so the
        // struct is the platform's, not a hand-written approximation.
        unsafe {
            let mut stat: libc::statfs = std::mem::zeroed();
            if libc::statfs(c_path.as_ptr(), &mut stat) != 0 {
                return None;
            }
            // Linux reports u64 blocks, macOS reports u32; normalise both.
            #[cfg(target_os = "linux")]
            let bytes = stat.f_bavail as u64 * stat.f_frsize as u64;
            #[cfg(target_os = "macos")]
            let bytes = stat.f_bavail as u64 * stat.f_bsize as u64;
            Some(bytes)
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Other platforms report a value or nothing; guessing would be worse
        // than admitting the measurement is unavailable.
        let _ = path;
        None
    }
}

/// 累计操作仍保留在隔离区的字节。
/// 参数：engine 为共享引擎；scope_id 为累计隔离记录的范围。
/// 返回：恢复引用对应的保留字节数或存储错误。
/// Sums the bytes a completed operation still holds in quarantine.
pub fn quarantine_retained_bytes(
    engine: &std::sync::Arc<Engine>,
    scope_id: &ScopeId,
) -> Result<u64, OpsError> {
    let control = engine.control_store()?;
    let mut total = 0u64;
    for operation in control.list_operations(scope_id, 1_000)? {
        for item in control.operation_items(&operation.operation_id)? {
            if item.result != diskgraph_store::OperationItemResult::Quarantined {
                continue;
            }
            if let Some(recovery_ref) = item.recovery_ref
                && let Ok(entry) = control.recovery(&recovery_ref)
                && entry.state == diskgraph_store::RecoveryState::Available
            {
                total = total.saturating_add(quarantine_bytes(&entry));
            }
        }
    }
    Ok(total)
}

/// 读取单个可用恢复条目的隔离对象长度。
/// 参数：entry 为恢复条目，含隔离定位键。
/// 返回：隔离对象元数据长度；定位或元数据读取失败时为零，不递归遍历目录。
pub(super) fn quarantine_bytes(entry: &diskgraph_store::RecoveryEntry) -> u64 {
    unhex_key(&entry.quarantine_locator)
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}
