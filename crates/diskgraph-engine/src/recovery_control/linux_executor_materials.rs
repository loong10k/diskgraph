use super::{ControlError, linux_message_credentials};
use crate::recovery_slot::{LinuxSupervisorMaterials, LinuxSupervisorPeer};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const MAGIC: &[u8; 8] = b"DGEX01A\n";

/// 原executor出生通道的单次摘要认证；来源：PF-06/Linux逐消息内核凭据，无Java对等对象。
/// 不交付任何目录/namespace/槽句柄；JSON摘要与数字身份仍须由原可信出生层绑定。
pub struct LinuxExecutorMaterials {
    socket: UnixDatagram,
    peer: Option<LinuxSupervisorPeer>,
    session: [u8; 32],
    deadline: Instant,
    used: bool,
    failure: Option<ControlError>,
}

impl LinuxExecutorMaterials {
    /// 参数：socket为原未命名接收端；返回：发送前启用SO_PASSCRED的同一端点或拒绝。
    /// 仅设置内核接收策略，不认证发送者、镜像或客户身份。
    pub fn prepare_receiver(socket: UnixDatagram) -> Result<UnixDatagram, ControlError> {
        Self::check_endpoint(&socket)?;
        LinuxSupervisorMaterials::prepare_receiver(socket)
    }

    /// 参数：socket为原已准备接收端，peer/session为原父和原JSON摘要，deadline为原绝对期限。
    /// 返回：独占接收责任；不接受未准备或命名端点，不采用消息内容自认证。
    pub fn receiver(
        socket: UnixDatagram,
        peer: LinuxSupervisorPeer,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        let mut enabled: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&enabled) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&mut enabled as *mut libc::c_int).cast(),
                &mut length,
            )
        } != 0
            || enabled != 1
            || length as usize != std::mem::size_of_val(&enabled)
        {
            return Err(ControlError::Protocol);
        }
        Self::new(socket, Some(peer), session, deadline)
    }

    /// 参数：socket为原未命名发送端，session为原JSON摘要，deadline为原期限；返回：单次发送责任。
    /// 不交付SCM_RIGHTS；调用者必须保留原executor的实际child owner和guardian容量责任。
    pub fn sender(
        socket: UnixDatagram,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        Self::new(socket, None, session, deadline)
    }

    fn new(
        socket: UnixDatagram,
        peer: Option<LinuxSupervisorPeer>,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        Self::check_endpoint(&socket)?;
        if Instant::now() >= deadline {
            return Err(ControlError::Deadline);
        }
        socket
            .set_nonblocking(true)
            .map_err(|_| ControlError::Unconfirmed)?;
        Ok(Self {
            socket,
            peer,
            session,
            deadline,
            used: false,
            failure: None,
        })
    }

    fn check_endpoint(socket: &UnixDatagram) -> Result<(), ControlError> {
        if !socket
            .local_addr()
            .map_err(|_| ControlError::Protocol)?
            .is_unnamed()
            || !socket
                .peer_addr()
                .map_err(|_| ControlError::Protocol)?
                .is_unnamed()
        {
            return Err(ControlError::Protocol);
        }
        for (option, expected) in [
            (libc::SO_TYPE, libc::SOCK_DGRAM),
            (libc::SO_DOMAIN, libc::AF_UNIX),
        ] {
            let mut actual: libc::c_int = 0;
            let mut length = std::mem::size_of_val(&actual) as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    option,
                    (&mut actual as *mut libc::c_int).cast(),
                    &mut length,
                )
            } != 0
                || length as usize != std::mem::size_of_val(&actual)
                || actual != expected
            {
                return Err(ControlError::Protocol);
            }
        }
        Ok(())
    }

    fn check(&self, cancel: &AtomicBool) -> Result<(), ControlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.used {
            return Err(ControlError::Protocol);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(ControlError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(ControlError::Deadline);
        }
        Ok(())
    }

    /// 参数：cancel为原取消；返回：true为唯一原子发送成功，false为本轮无进展，错误永久锁存。
    /// 不续传datagram；发送成功后的取消或超时也不得重发，不刷新原期限。
    pub fn poll_send(&mut self, cancel: &AtomicBool) -> Result<bool, ControlError> {
        let result = self.send_once(cancel);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }

    fn send_once(&mut self, cancel: &AtomicBool) -> Result<bool, ControlError> {
        self.check(cancel)?;
        if self.peer.is_some() {
            return Err(ControlError::Protocol);
        }
        let mut bytes = [0_u8; 40];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..].copy_from_slice(&self.session);
        let sent = unsafe {
            libc::send(
                self.socket.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if sent < 0 {
            return match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock => Ok(false),
                _ => Err(ControlError::Unconfirmed),
            };
        }
        // 先锁定已发送状态，再检查原期限，避免发送后错误诱发重复认证包。
        self.used = true;
        if sent != 40 {
            return Err(ControlError::Protocol);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(ControlError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(ControlError::Deadline);
        }
        Ok(true)
    }

    /// 参数：cancel为原取消；返回：true为原父凭据/会话认证成功，false为本轮无消息，错误锁存。
    /// 每轮仅一笔非阻塞recvmsg；意外FD先关闭，随后检查原取消/期限，绝不生成容量能力。
    pub fn poll_receive(&mut self, cancel: &AtomicBool) -> Result<bool, ControlError> {
        let result = self.receive_once(cancel);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }

    fn receive_once(&mut self, cancel: &AtomicBool) -> Result<bool, ControlError> {
        self.check(cancel)?;
        let peer = self.peer.as_ref().ok_or(ControlError::Protocol)?;
        let mut bytes = [0_u8; 41];
        let mut control = [0_usize; 128];
        let mut vector = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = std::mem::size_of_val(&control);
        let received = unsafe {
            libc::recvmsg(
                self.socket.as_raw_fd(),
                &mut message,
                libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
            )
        };
        if received < 0 {
            return match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock => Ok(false),
                _ => Err(ControlError::Unconfirmed),
            };
        }
        let credentials = linux_message_credentials::inspect_credentials(
            &message,
            control.as_ptr() as usize,
            std::mem::size_of_val(&control),
        );
        // 原内核已交付FD必须先关闭；取消或过期不能跳过处置，也不改变原拒绝优先级。
        self.check(cancel)?;
        let credentials = credentials?;
        if received != 40
            || &bytes[..8] != MAGIC
            || bytes[8..40] != self.session
            || !credentials
                .as_ref()
                .is_some_and(|actual| peer.matches(actual))
        {
            return Err(ControlError::Protocol);
        }
        self.used = true;
        Ok(true)
    }
}
