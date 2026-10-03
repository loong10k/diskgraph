//! ops_time：既有文件操作职责的原生 Rust 实现。

/// 读取控制记录的当前时间。
/// 参数：无。
/// 返回：自 Unix epoch 起的毫秒；系统时间错误时为零。
/// Milliseconds since the epoch for control-plane timestamps.
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
