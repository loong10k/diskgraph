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
    deadline: Option<Instant>,
    version: u16,
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
        Self::with_receiver(
            stream,
            session,
            Some(deadline),
            1,
            ControlReceiver::new(session, deadline),
        )
    }

    /// 参数：stream 为已认证原私有通道、session 为原会话；返回：版本2生命周期写入责任。
    /// 每条通知必须通过 send_lifecycle 提供有限交付期限；不采用启动期限或刷新请求权限。
    pub fn new_lifecycle(stream: UnixStream, session: [u8; 32]) -> Result<Self, ControlError> {
        Self::with_receiver(
            stream,
            session,
            None,
            2,
            ControlReceiver::new_lifecycle(session),
        )
    }

    fn with_receiver(
        stream: UnixStream,
        session: [u8; 32],
        deadline: Option<Instant>,
        version: u16,
        validation: ControlReceiver,
    ) -> Result<Self, ControlError> {
        stream
            .set_nonblocking(true)
            .map_err(|_| ControlError::Unconfirmed)?;
        Ok(Self {
            stream,
            session,
            deadline,
            version,
            sequence: 0,
            validation,
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
        if let Some(error) = self.failure {
            return Err(error);
        }
        let result = match self.deadline {
            Some(deadline) if self.version == 1 => self.send_inner(notification, cancel, deadline),
            _ => Err(ControlError::Protocol),
        };
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }

    /// 参数：notification 为生命周期事件、cancel 为取消标志、deadline 为本条有限交付期限。
    /// 返回：完整交付或永久锁存错误；仅版本2可用，不重置序列、预算或部分发送状态。
    pub fn send_lifecycle(
        &mut self,
        notification: ControlNotification,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), ControlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let result = if self.version == 2 {
            self.send_inner(notification, cancel, deadline)
        } else {
            Err(ControlError::Protocol)
        };
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }
    /// 参数：deadline 为已认证调用方原期限上界；返回：无，只能缩短原写入期限，不能续期。
    #[cfg(target_os = "linux")]
    pub(crate) fn restrict_deadline(&mut self, deadline: Instant) {
        self.deadline = Some(
            self.deadline
                .map_or(deadline, |original| original.min(deadline)),
        );
    }

    fn check(&self, cancel: &AtomicBool, deadline: Instant) -> Result<(), ControlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(ControlError::Cancelled);
        }
        if Instant::now() >= deadline
            || self
                .deadline
                .is_some_and(|original| Instant::now() >= original)
        {
            return Err(ControlError::Deadline);
        }
        Ok(())
    }
    fn send_inner(
        &mut self,
        notification: ControlNotification,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), ControlError> {
        self.check(cancel, deadline)?;
        let bytes = ControlFrame {
            version: self.version,
            session: self.session,
            sequence: self.sequence,
            notification,
        }
        .encode()?;
        // 原累计预算及状态先验证；部分写入失败不能回滚状态后重新发送。
        self.validation.push(&bytes)?;
        let mut offset = 0;
        while offset < bytes.len() {
            self.check(cancel, deadline)?;
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
                        self.check(cancel, deadline)?;
                        std::thread::sleep(
                            deadline
                                .saturating_duration_since(Instant::now())
                                .min(std::time::Duration::from_millis(1)),
                        );
                    }
                    std::io::ErrorKind::Interrupted => {}
                    _ => return Err(ControlError::Unconfirmed),
                }
            }
            self.check(cancel, deadline)?;
        }
        self.sequence = self.sequence.checked_add(1).ok_or(ControlError::Budget)?;
        self.check(cancel, deadline)
    }
}
