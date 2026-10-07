use crate::EngineError;
/// 本次原生时钟及实际 domain；来源：PF-06 原期限桥接，无 Java 对等对象。
pub(super) struct ClockSample {
    pub(super) nanos: u64,
    pub(super) margin_nanos: u64,
    pub(super) domain: [u64; 3],
}
impl ClockSample {
    /// 参数：无；返回：本机真实原生采样及域，失败拒绝，不用 UTC 或用户配置回退。
    pub(super) fn read() -> Result<Self, EngineError> {
        #[cfg(target_os = "linux")]
        {
            super::linux_clock::read()
        }
        #[cfg(target_os = "macos")]
        {
            super::macos_clock::read()
        }
        #[cfg(windows)]
        {
            super::windows_clock::read()
        }
    }
}
