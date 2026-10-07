use super::{ControlError, ControlFrame, ControlReceiver};
use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 原已认证 Unix 通道的有界读取责任；来源：PF-06 私有监督协议，无 Java 对等对象。
/// 不建立对端信任，不从控制通知推定原资源回收；错误永久锁存。
pub struct UnixControlReader {
    stream: UnixStream,
    receiver: ControlReceiver,
    deadline: Instant,
    pending: VecDeque<ControlFrame>,
    failure: Option<ControlError>,
    ended: bool,
}
impl UnixControlReader {
    /// 参数：stream 为唯一未克隆的已认证私有通道，session 和 deadline 为原会话及绝对期限。
    /// 返回：非阻塞读取责任或原通道错误；设置模式作用于原 socket，不接受公开 stdio。
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
            receiver: ControlReceiver::new(session, deadline),
            deadline,
            pending: VecDeque::new(),
            failure: None,
            ended: false,
        })
    }
    /// 参数：cancel 为原取消标志；返回：下一完整帧，已确认协议 EOF，或锁存的固定错误。
    /// None 仅表示完整 CleanupComplete 后的实际 EOF，不证明原 wait/Job/I/O/目录已回收。
    pub fn receive(&mut self, cancel: &AtomicBool) -> Result<Option<ControlFrame>, ControlError> {
        let result = self.receive_inner(cancel);
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
    fn receive_inner(&mut self, cancel: &AtomicBool) -> Result<Option<ControlFrame>, ControlError> {
        loop {
            self.check(cancel)?;
            if let Some(frame) = self.pending.pop_front() {
                return Ok(Some(frame));
            }
            if self.ended {
                return Ok(None);
            }
            // 每轮固定缓冲；解析器同时扣原累计字节/帧预算，部分帧不会刷新期限。
            let mut buffer = [0_u8; 4096];
            let read = self.stream.read(&mut buffer);
            self.check(cancel)?;
            match read {
                Ok(0) => {
                    self.receiver.end_of_stream()?;
                    self.ended = true;
                }
                Ok(count) => {
                    let frames = self.receiver.push(&buffer[..count])?;
                    self.pending
                        .try_reserve(frames.len())
                        .map_err(|_| ControlError::Budget)?;
                    self.pending.extend(frames);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    self.check(cancel)?;
                    std::thread::sleep(
                        self.deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(1)),
                    );
                }
                Err(_) => return Err(ControlError::Unconfirmed),
            }
        }
    }
}
