//! 证据采样时钟。

use std::time::{SystemTime, UNIX_EPOCH};

/// 读取实时证据采样时钟。
/// 参数：无。
/// 返回：Unix 毫秒；epoch 错误保留零值。
pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
