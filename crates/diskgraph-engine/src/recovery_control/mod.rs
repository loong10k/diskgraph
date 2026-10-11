//! 私有恢复控制协议；协议校验不证明原生资源回收或前端已经有限退出。
#[cfg(target_os = "linux")]
mod frontend_read;
#[cfg(target_os = "linux")]
mod linux_frontend_connection;
#[cfg(target_os = "linux")]
mod linux_peer_unix_identity;
#[cfg(target_os = "linux")]
pub use linux_peer_unix_identity::LinuxPeerUnixIdentity;
#[cfg(all(test, target_os = "linux"))]
mod linux_frontend_connection_tests;
#[cfg(target_os = "linux")]
pub(crate) mod linux_message_credentials;
#[cfg(target_os = "linux")]
pub use frontend_read::FrontendRead;
#[cfg(target_os = "linux")]
pub use linux_frontend_connection::LinuxFrontendConnection;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_foreground_output;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix_foreground_output::UnixForegroundOutput;
mod control_error;
mod control_frame;
mod control_notification;
mod control_phase;
mod control_receiver;
#[cfg(test)]
mod lifecycle_control_tests;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod supervisor_control_lifecycle_tests;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod unix_foreground_output_tests;
pub use control_error::ControlError;
pub use control_frame::ControlFrame;
pub use control_notification::ControlNotification;
pub use control_receiver::ControlReceiver;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_control_writer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix_control_writer::UnixControlWriter;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_control_reader;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix_control_reader::UnixControlReader;

#[cfg(target_os = "linux")]
mod linux_control_read;
#[cfg(target_os = "linux")]
mod linux_control_reader;
#[cfg(target_os = "linux")]
pub use linux_control_read::LinuxControlRead;
#[cfg(target_os = "linux")]
pub use linux_control_reader::LinuxControlReader;

#[cfg(all(test, target_os = "linux"))]
mod linux_control_reader_tests;
