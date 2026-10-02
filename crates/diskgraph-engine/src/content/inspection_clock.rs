//! 内容采样时钟保留独立的错误语义。

/// 读取检查日志的毫秒采样时钟。
/// 参数：无。
/// 返回：Unix 毫秒；早于 epoch 时保留既有零值。
/// Milliseconds since the epoch for logs.
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
