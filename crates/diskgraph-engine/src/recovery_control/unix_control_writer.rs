use super::{ControlError, ControlFrame, ControlNotification, ControlReceiver};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
/// 原已认证 Unix socket 的有界通知写入；来源：PF-06，无 Java 对等对象。
/// 不认证 peer、不释放原资源；部分失败后禁止重发，原监督责任必须独立保留。
pub struct UnixControlWriter {
    stream: UnixStream,
    session: [u8; 32],
    deadline: Instant,
    sequence: u64,
    validation: ControlReceiver,
    failure: Option<ControlError>,
}
impl UnixControlWriter {
    /// 参数：原已认证 stream、session 及绝对期限；返回：不继承公开输出的唯一非阻塞写入责任。
    /// 调用方必须提供未克隆、未向其他 owner 借出的私有 stream；设置模式作用于原 socket。
    pub fn new(
        stream: UnixStream,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        stream
            .set_nonblocking(true)
            .map_err(|_| ControlError::Unconfirmed)?;
        Ok(Self {
            stream,
            session,
            deadline,
            sequence: 0,
            validation: ControlReceiver::new(session, deadline),
            failure: None,
        })
    }
    /// 参数：notification 为原监督状态、cancel 为原取消；返回：完整通知交付或锁存固定失败。
    /// 每次 send 使用原生 DONTWAIT/NOSIGNAL；原 socket 已明确设为非阻塞，内核单次调用不承诺硬墙钟上限。
    pub fn send(
        &mut self,
        notification: ControlNotification,
        cancel: &AtomicBool,
    ) -> Result<(), ControlError> {
        let result = self.send_inner(notification, cancel);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }
    fn check(&self, cancel: &AtomicBool) -> Result<(), ControlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(ControlError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(ControlError::Deadline);
        }
        Ok(())
    }
    fn send_inner(
        &mut self,
        notification: ControlNotification,
        cancel: &AtomicBool,
    ) -> Result<(), ControlError> {
        self.check(cancel)?;
        let bytes = ControlFrame {
            version: 1,
            session: self.session,
            sequence: self.sequence,
            notification,
        }
        .encode()?;
        // 原累计预算及状态先验证；部分写入失败不能回滚状态后重新发送。
        self.validation.push(&bytes)?;
        let mut offset = 0;
        while offset < bytes.len() {
            self.check(cancel)?;
            let wrote = unsafe {
                libc::send(
                    self.stream.as_raw_fd(),
                    bytes[offset..].as_ptr().cast(),
                    bytes.len() - offset,
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                )
            };
            if wrote > 0 {
                offset += usize::try_from(wrote).map_err(|_| ControlError::Unconfirmed)?;
            } else if wrote == 0 {
                return Err(ControlError::Unconfirmed);
            } else {
                let error = std::io::Error::last_os_error();
                match error.kind() {
                    std::io::ErrorKind::WouldBlock => {
                        self.check(cancel)?;
                        std::thread::sleep(
                            self.deadline
                                .saturating_duration_since(Instant::now())
                                .min(std::time::Duration::from_millis(1)),
                        );
                    }
                    std::io::ErrorKind::Interrupted => {}
                    _ => return Err(ControlError::Unconfirmed),
                }
            }
            self.check(cancel)?;
        }
        self.sequence = self.sequence.checked_add(1).ok_or(ControlError::Budget)?;
        self.check(cancel)
    }
}
