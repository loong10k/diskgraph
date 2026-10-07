//! 私有恢复控制协议；协议校验不证明原生资源回收或前端已经有限退出。
mod control_error;
mod control_frame;
mod control_notification;
mod control_phase;
mod control_receiver;
pub use control_error::ControlError;
pub use control_frame::ControlFrame;
pub use control_notification::ControlNotification;
pub use control_receiver::ControlReceiver;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_control_writer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix_control_writer::UnixControlWriter;
