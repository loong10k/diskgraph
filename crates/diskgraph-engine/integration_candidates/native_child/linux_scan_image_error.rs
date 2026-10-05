use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 非Clone的原检查点错误，计数真实析构以发现重建或丢失。
/// 来源：原生 Rust io::Error 自定义载荷所有权验收。
#[derive(Debug)]
pub(super) struct LinuxScanImageError {
    pub(super) drops: Arc<AtomicUsize>,
}

impl std::fmt::Display for LinuxScanImageError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str("original-sealed-image-checkpoint")
    }
}
impl std::error::Error for LinuxScanImageError {}
impl Drop for LinuxScanImageError {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
