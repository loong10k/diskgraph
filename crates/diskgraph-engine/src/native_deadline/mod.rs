//! 仅同一次出生的已认证本机私有通道使用，时间材料不建立对端信任。
mod clock_sample;
mod clock_stamp;
#[cfg(target_os = "linux")]
mod linux_clock;
#[cfg(target_os = "macos")]
mod macos_clock;
#[cfg(windows)]
mod windows_clock;
pub use clock_stamp::ClockStamp;
