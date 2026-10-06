use super::control_phase::ControlPhase;
use super::{ControlError, ControlFrame, ControlNotification};
use std::time::Instant;

/// 绑定原会话与绝对期限的私有通知接收器；来源：PF-06，无 Java 对等对象。
/// 上限为每帧 4096、累计 65536 字节与 64 帧；不取得或释放原生资源 owner。
pub struct ControlReceiver {
    session: [u8; 32],
    deadline: Instant,
    charged: usize,
    sequence: u64,
    phase: ControlPhase,
    failure: Option<ControlError>,
    header: [u8; 4],
    header_len: usize,
    body: Vec<u8>,
    body_len: usize,
}

impl ControlReceiver {
    /// 参数：session 为已认证监督方单向私有通道的原会话，deadline 为原绝对期限；返回：接收器。
    /// 不得用于接受前端命令；本接收器不认证 peer，也不将通知转换为物理回收证明。
    pub fn new(session: [u8; 32], deadline: Instant) -> Self {
        Self {
            session,
            deadline,
            charged: 0,
            sequence: 0,
            phase: ControlPhase::AwaitReady,
            failure: None,
            header: [0; 4],
            header_len: 0,
            body: Vec::new(),
            body_len: 0,
        }
    }

    /// 参数：bytes 为私有控制通道读出的字节；返回：完整通知或锁存的固定错误。
    /// 一次错误后不接受后续合法帧；外部调用方仍必须保留原恢复责任。
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<ControlFrame>, ControlError> {
        let result = self.push_inner(bytes);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }

    fn check(&self) -> Result<(), ControlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if Instant::now() >= self.deadline {
            return Err(ControlError::Deadline);
        }
        Ok(())
    }

    fn push_inner(&mut self, mut bytes: &[u8]) -> Result<Vec<ControlFrame>, ControlError> {
        self.check()?;
        if self.phase == ControlPhase::Complete {
            return Err(ControlError::Protocol);
        }
        // 前缀、部分正文、重复 Pending 和无效帧都消费原累计预算。
        self.charged = self
            .charged
            .checked_add(bytes.len())
            .ok_or(ControlError::Budget)?;
        if self.charged > 65536 {
            return Err(ControlError::Budget);
        }
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            self.check()?;
            if self.sequence >= 64 {
                return Err(ControlError::Budget);
            }
            if self.phase == ControlPhase::Complete {
                return Err(ControlError::Protocol);
            }
            if self.header_len < 4 {
                let take = (4 - self.header_len).min(bytes.len());
                self.header[self.header_len..self.header_len + take]
                    .copy_from_slice(&bytes[..take]);
                self.header_len += take;
                bytes = &bytes[take..];
                if self.header_len < 4 {
                    break;
                }
                self.body_len = u32::from_be_bytes(self.header) as usize;
                if self.body_len > 4096 {
                    return Err(ControlError::Budget);
                }
                if self.body_len == 0 {
                    return Err(ControlError::Protocol);
                }
                // 先验证声明长度才分配，保留最多一帧正文，不加载完整流。
                self.body
                    .try_reserve(self.body_len)
                    .map_err(|_| ControlError::Budget)?;
            }
            let take = (self.body_len - self.body.len()).min(bytes.len());
            self.body.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.body.len() < self.body_len {
                break;
            }
            let frame: ControlFrame =
                serde_json::from_slice(&self.body).map_err(|_| ControlError::Protocol)?;
            self.check()?;
            self.accept(&frame)?;
            frames.try_reserve(1).map_err(|_| ControlError::Budget)?;
            frames.push(frame);
            self.header_len = 0;
            self.body_len = 0;
            self.body.clear();
        }
        self.check()?;
        Ok(frames)
    }

    fn accept(&mut self, frame: &ControlFrame) -> Result<(), ControlError> {
        if frame.version != 1 || frame.session != self.session || frame.sequence != self.sequence {
            return Err(ControlError::Protocol);
        }
        self.phase = match (&self.phase, &frame.notification) {
            (ControlPhase::AwaitReady, ControlNotification::Ready {}) => ControlPhase::Running,
            (ControlPhase::Running, ControlNotification::ForegroundEnded { .. }) => {
                ControlPhase::ForegroundEnded
            }
            (
                ControlPhase::ForegroundEnded | ControlPhase::Recovering,
                ControlNotification::CleanupPending {},
            ) => ControlPhase::Recovering,
            (
                ControlPhase::ForegroundEnded | ControlPhase::Recovering,
                ControlNotification::CleanupComplete {},
            ) => ControlPhase::Complete,
            _ => return Err(ControlError::Protocol),
        };
        self.sequence += 1;
        Ok(())
    }

    /// 参数：无；返回：完整通知流结束或未确认错误；断开不能代替回收完成。
    /// 即使已收到 ForegroundEnded，未收到原监督方的 CleanupComplete 仍返回 Unconfirmed。
    pub fn end_of_stream(&mut self) -> Result<(), ControlError> {
        let result = self.check().and_then(|()| {
            if self.header_len == 0 && self.phase == ControlPhase::Complete {
                Ok(())
            } else {
                Err(ControlError::Unconfirmed)
            }
        });
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }
}
