use super::ChildError;
use std::io;

/// 仅在同一 native64 父子启动 socket 传输的固定12字节消息，不是外部worker协议。
/// 来源：原生 Rust/C Init、Ready、ACK 与原errno握手。
#[repr(C)]
pub(super) struct LinuxAtomicMessage {
    pub(super) kind: u32,
    pub(super) phase: u32,
    pub(super) error: i32,
}

impl LinuxAtomicMessage {
    /// 参数：fd 是唯一owner的 nonblocking SOCK_SEQPACKET，kind 是固定Init/ACK；返回：一笔完成或Pending。
    pub(super) fn send(fd: i32, kind: u32) -> Result<bool, ChildError> {
        let message = Self {
            kind,
            phase: 0,
            error: 0,
        };
        let result = unsafe {
            libc::send(
                fd,
                (&message as *const Self).cast(),
                std::mem::size_of::<Self>(),
                libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Ok(false);
            }
            return Err(ChildError::io("send atomic startup permit", error));
        }
        if result as usize != std::mem::size_of::<Self>() {
            return Err(ChildError::io(
                "send atomic startup permit",
                io::Error::new(io::ErrorKind::WriteZero, "incomplete startup packet"),
            ));
        }
        Ok(true)
    }

    /// 参数：fd 为原 socket；返回：Pending=None，EOF=Some(None)，完整固定消息=Some(Some)，截断失败。
    pub(super) fn receive(fd: i32) -> Result<Option<Option<Self>>, ChildError> {
        let mut message = Self {
            kind: 0,
            phase: 0,
            error: 0,
        };
        let result = unsafe {
            libc::recv(
                fd,
                (&mut message as *mut Self).cast(),
                std::mem::size_of::<Self>(),
                libc::MSG_DONTWAIT | libc::MSG_TRUNC,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Ok(None);
            }
            return Err(ChildError::io("receive atomic startup fact", error));
        }
        if result == 0 {
            return Ok(Some(None));
        }
        if result as usize != std::mem::size_of::<Self>() {
            return Err(ChildError::io(
                "receive atomic startup fact",
                io::Error::new(io::ErrorKind::InvalidData, "truncated startup packet"),
            ));
        }
        Ok(Some(Some(message)))
    }

    /// 参数：self 是已收到的有界消息；返回：合法原生失败或固定Ready，拒绝未知字段组合。
    pub(super) fn verify(self) -> Result<(), ChildError> {
        if self.kind == 1 && self.phase == 0 && self.error == 0 {
            return Ok(());
        }
        if self.kind == 2 && (1..=11).contains(&self.phase) && (1..=4095).contains(&self.error) {
            let context = match self.phase {
                1 => "atomic child dup3",
                2 => "atomic child close_range",
                3 => "atomic child reset handler",
                4 => "atomic child reset mask",
                5 => "atomic child Init",
                6 => "atomic child setsid",
                7 => "atomic child no_new_privs",
                8 => "atomic child seccomp",
                9 => "atomic child Ready",
                10 => "atomic child ACK",
                _ => "atomic child execveat",
            };
            return Err(ChildError::io(
                context,
                io::Error::from_raw_os_error(self.error),
            ));
        }
        Err(ChildError::io(
            "atomic startup protocol",
            io::Error::new(io::ErrorKind::InvalidData, "unexpected startup message"),
        ))
    }
}
