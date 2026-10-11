use super::{ControlError, ControlFrame, ControlReceiver, LinuxControlRead};
use crate::recovery_slot::LinuxSupervisorPeer;
use std::collections::VecDeque;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// 持有原私有接收端并逐笔核对内核发送凭据；来源：Linux recvmsg／PF-06，无 Java 对等对象。
/// 原 child owner、镜像资格和 bootstrap 必须另由可信 broker 保留，帧与 EOF 均不代替实际 wait。
pub struct LinuxControlReader {
    stream: UnixStream,
    peer: LinuxSupervisorPeer,
    receiver: ControlReceiver,
    deadline: Option<Instant>,
    pending: VecDeque<ControlFrame>,
    failure: Option<ControlError>,
    ended: bool,
}

impl LinuxControlReader {
    /// 参数：stream 为出生前原未命名 socketpair 接收端；返回：启用逐消息凭据的同一端点。
    /// 必须先于发送执行；不签发原进程身份，不复制通道或提供公开监听入口。
    pub fn prepare_receiver(stream: UnixStream) -> Result<UnixStream, ControlError> {
        check_endpoint(&stream)?;
        let enabled: libc::c_int = 1;
        if unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as libc::socklen_t,
            )
        } != 0
        {
            return Err(ControlError::Unconfirmed);
        }
        Ok(stream)
    }

    /// 参数：stream 为原已准备端点，peer 为原 child owner 绑定的内核身份，session/deadline 不刷新。
    /// 返回：独占非阻塞读取责任；拒绝未启用逐消息凭据的端点，不采用创建者凭据自认证。
    pub fn new(
        stream: UnixStream,
        peer: LinuxSupervisorPeer,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        Self::with_receiver(
            stream,
            peer,
            Some(deadline),
            ControlReceiver::new(session, deadline),
        )
    }

    /// 参数：stream 为出生前已准备的原私有端点，peer/session 为原内核身份及会话。
    /// 返回：显式版本2生命周期读取责任；每次poll仅一笔非阻塞读取，不刷新公共请求期限。
    /// 不得在旧协议超时后重建；仍须保留唯一原child与ACTIVE至实际退休。
    pub fn new_lifecycle(
        stream: UnixStream,
        peer: LinuxSupervisorPeer,
        session: [u8; 32],
    ) -> Result<Self, ControlError> {
        Self::with_receiver(stream, peer, None, ControlReceiver::new_lifecycle(session))
    }

    fn with_receiver(
        stream: UnixStream,
        peer: LinuxSupervisorPeer,
        deadline: Option<Instant>,
        receiver: ControlReceiver,
    ) -> Result<Self, ControlError> {
        check_endpoint(&stream)?;
        let mut enabled: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&enabled) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
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
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(ControlError::Deadline);
        }
        Ok(Self {
            stream,
            peer,
            receiver,
            deadline,
            pending: VecDeque::new(),
            failure: None,
            ended: false,
        })
    }

    /// 参数：cancel 为原取消标志；返回：一笔有界读取后的 Pending、完整帧、确认 EOF 或锁存原错误。
    /// 不等待、不刷新期限；附加 FD 先关闭再拒绝，任何帧解析前均核对实际内核发送者。
    pub fn poll(&mut self, cancel: &AtomicBool) -> Result<LinuxControlRead, ControlError> {
        let result = self.poll_inner(cancel);
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
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(ControlError::Deadline);
        }
        Ok(())
    }

    fn poll_inner(&mut self, cancel: &AtomicBool) -> Result<LinuxControlRead, ControlError> {
        self.check(cancel)?;
        if let Some(frame) = self.pending.pop_front() {
            return Ok(LinuxControlRead::Frame(frame));
        }
        if self.ended {
            return Ok(LinuxControlRead::ConfirmedEof);
        }
        let mut bytes = [0_u8; 4096];
        // usize 数组保证 cmsghdr 对齐，固定控制缓冲限制每轮成本；截断由内核标记并拒绝。
        let mut control = [0_usize; 32];
        let mut vector = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = std::mem::size_of_val(&control);
        let count = unsafe {
            libc::recvmsg(
                self.stream.as_raw_fd(),
                &mut message,
                libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
            )
        };
        if count < 0 {
            let error = std::io::Error::last_os_error();
            self.check(cancel)?;
            return match error.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => {
                    Ok(LinuxControlRead::Pending)
                }
                _ => Err(ControlError::Unconfirmed),
            };
        }
        // 接收后先处置所有已交付 FD；即使此时取消/过期或凭据错误也不能泄漏。
        let mut credentials = None;
        let mut valid = true;
        let mut header = unsafe { libc::CMSG_FIRSTHDR(&message) };
        while !header.is_null() {
            let base = unsafe { libc::CMSG_LEN(0) } as usize;
            let length = unsafe { (*header).cmsg_len };
            let offset = (header as usize).checked_sub(control.as_ptr() as usize);
            if length < base
                || !offset.is_some_and(|start| {
                    start.checked_add(length).is_some_and(|end| {
                        end <= message.msg_controllen && end <= std::mem::size_of_val(&control)
                    })
                })
            {
                valid = false;
                break;
            }
            let length = length - base;
            let level = unsafe { (*header).cmsg_level };
            let kind = unsafe { (*header).cmsg_type };
            let data = unsafe { libc::CMSG_DATA(header) };
            if level == libc::SOL_SOCKET && kind == libc::SCM_RIGHTS {
                valid = false;
                for index in 0..length / std::mem::size_of::<libc::c_int>() {
                    let fd =
                        unsafe { std::ptr::read_unaligned(data.cast::<libc::c_int>().add(index)) };
                    if fd >= 0 {
                        drop(unsafe { OwnedFd::from_raw_fd(fd) });
                    }
                }
            } else if level == libc::SOL_SOCKET
                && kind == libc::SCM_CREDENTIALS
                && length == std::mem::size_of::<libc::ucred>()
            {
                if credentials.is_some() {
                    valid = false;
                }
                credentials = Some(unsafe { std::ptr::read_unaligned(data.cast::<libc::ucred>()) });
            } else {
                valid = false;
            }
            header = unsafe { libc::CMSG_NXTHDR(&message, header) };
        }
        self.check(cancel)?;
        if !valid || message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0 {
            return Err(ControlError::Protocol);
        }
        if count == 0 {
            self.receiver.end_of_stream()?;
            self.ended = true;
            return Ok(LinuxControlRead::ConfirmedEof);
        }
        if !credentials
            .as_ref()
            .is_some_and(|actual| self.peer.matches(actual))
        {
            return Err(ControlError::Protocol);
        }
        let frames = self.receiver.push(&bytes[..count as usize])?;
        self.pending
            .try_reserve(frames.len())
            .map_err(|_| ControlError::Budget)?;
        self.pending.extend(frames);
        self.check(cancel)?;
        Ok(self
            .pending
            .pop_front()
            .map_or(LinuxControlRead::Pending, LinuxControlRead::Frame))
    }
}

fn check_endpoint(stream: &UnixStream) -> Result<(), ControlError> {
    if !stream
        .local_addr()
        .map_err(|_| ControlError::Protocol)?
        .is_unnamed()
        || !stream
            .peer_addr()
            .map_err(|_| ControlError::Protocol)?
            .is_unnamed()
    {
        return Err(ControlError::Protocol);
    }
    // 原 broker 通道不得在后续 exec 时泄漏；保留原标志，不复制 socket。
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) };
    if flags < 0
        || unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0
    {
        return Err(ControlError::Unconfirmed);
    }
    stream
        .set_nonblocking(true)
        .map_err(|_| ControlError::Unconfirmed)
}
