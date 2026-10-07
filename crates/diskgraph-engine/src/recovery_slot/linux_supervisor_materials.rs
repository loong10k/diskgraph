use super::{LinuxSupervisorPeer, LinuxSupervisorTrust};
use crate::recovery_control::ControlError;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"DGSM01A\n";

/// 出生时私有 socketpair 的一次性原句柄交付；来源：Linux SCM_RIGHTS/SCM_CREDENTIALS / PF-06。
/// 无 Java 对等对象。只交付 root/user/mount 三个句柄，不转移槽锁或签发发行信任。
/// 调用方必须提供原未克隆通道、受信发送进程 owner、会话及原期限；随机会话不代替出生认证。
pub struct LinuxSupervisorMaterials {
    socket: UnixDatagram,
    peer: LinuxSupervisorPeer,
    session: [u8; 32],
    deadline: Instant,
    used: bool,
    failure: Option<ControlError>,
    receiving: bool,
}

impl LinuxSupervisorMaterials {
    /// 出生前准备唯一接收端。
    /// 参数：socket 为原 socketpair 接收端；返回：配置后的同一原 socket，或通道/配置拒绝。
    /// 必须在任何发送之前调用；不认证宿主、签发身份或转移恢复责任。
    pub fn prepare_receiver(socket: UnixDatagram) -> Result<UnixDatagram, ControlError> {
        if !socket
            .peer_addr()
            .map_err(|_| ControlError::Protocol)?
            .is_unnamed()
        {
            return Err(ControlError::Protocol);
        }
        socket
            .set_nonblocking(true)
            .map_err(|_| ControlError::Unconfirmed)?;
        let enabled: libc::c_int = 1;
        if unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as libc::socklen_t,
            )
        } != 0
        {
            return Err(ControlError::Unconfirmed);
        }
        Ok(socket)
    }

    /// 参数：socket 为原未命名已连接私有 socketpair，peer/session 为原出生绑定，deadline 不刷新。
    /// 返回：启用内核逐消息凭据的非阻塞责任；不接受命名监听地址或公开 stdio。
    pub fn new(
        socket: UnixDatagram,
        peer: LinuxSupervisorPeer,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        Self::with_direction(socket, peer, session, deadline, true)
    }

    /// 构造单向发送端。
    /// 参数：socket 为原通道，peer/session 为原对端与会话，deadline 为原期限。
    /// 返回：不可接收的发送责任，或通道、方向、会话及期限拒绝。
    /// 发送端不启用 SO_PASSCRED，避免首包发送自动绑定抽象地址破坏原未命名通道契约。
    pub fn new_sender(
        socket: UnixDatagram,
        peer: LinuxSupervisorPeer,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Self, ControlError> {
        Self::with_direction(socket, peer, session, deadline, false)
    }

    fn with_direction(
        socket: UnixDatagram,
        peer: LinuxSupervisorPeer,
        session: [u8; 32],
        deadline: Instant,
        receiving: bool,
    ) -> Result<Self, ControlError> {
        if !socket
            .peer_addr()
            .map_err(|_| ControlError::Protocol)?
            .is_unnamed()
        {
            return Err(ControlError::Protocol);
        }
        socket
            .set_nonblocking(true)
            .map_err(|_| ControlError::Unconfirmed)?;
        let enabled: libc::c_int = i32::from(receiving);
        if unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as libc::socklen_t,
            )
        } != 0
        {
            return Err(ControlError::Unconfirmed);
        }
        Ok(Self {
            socket,
            peer,
            session,
            deadline,
            used: false,
            failure: None,
            receiving,
        })
    }

    /// 参数：trust 为原宿主对象、cancel 为原取消；返回：完整三句柄交付或永久锁存失败。
    /// 源对象始终由宿主保留；失败/取消不刷新预算，不重发可能已交付的包。
    pub fn send(
        &mut self,
        trust: &LinuxSupervisorTrust,
        cancel: &AtomicBool,
    ) -> Result<(), ControlError> {
        let result = self.send_inner(trust, cancel);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }

    /// 参数：cancel 为原取消；返回：完整且匹配原发送凭据的三个原对象引用。
    /// 截断、错误凭据、错误会话、额外句柄和超时均关闭已接收句柄，不生成部署资格。
    pub fn receive(&mut self, cancel: &AtomicBool) -> Result<LinuxSupervisorTrust, ControlError> {
        let result = self.receive_inner(cancel);
        if let Err(error) = &result {
            self.failure = Some(*error);
        }
        result
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

    fn pause(&self) {
        std::thread::sleep(
            self.deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(1)),
        );
    }

    fn send_inner(
        &mut self,
        trust: &LinuxSupervisorTrust,
        cancel: &AtomicBool,
    ) -> Result<(), ControlError> {
        if self.receiving {
            return Err(ControlError::Protocol);
        }
        let mut bytes = [0_u8; 40];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..].copy_from_slice(&self.session);
        let files = [
            trust.root.as_raw_fd(),
            trust.user_namespace.as_raw_fd(),
            trust.mount_namespace.as_raw_fd(),
        ];
        let mut control = [0_usize; 16];
        let mut vector = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen =
            unsafe { libc::CMSG_SPACE(std::mem::size_of_val(&files) as u32) } as usize;
        let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
        // 控制缓冲按 usize 对齐；ABI 长度使用 libc 宏，不按裸结构长度猜测填充。
        unsafe {
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(&files) as u32) as usize;
            std::ptr::copy_nonoverlapping(
                files.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(header),
                std::mem::size_of_val(&files),
            );
        }
        loop {
            self.check(cancel)?;
            let sent = unsafe {
                libc::sendmsg(
                    self.socket.as_raw_fd(),
                    &message,
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                )
            };
            if sent >= 0 {
                // 原子 datagram 不续传；先锁住重复发送，再报告取消或长度异常。
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
                return Ok(());
            }
            match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::Interrupted => {}
                std::io::ErrorKind::WouldBlock => self.pause(),
                _ => return Err(ControlError::Unconfirmed),
            }
        }
    }

    fn receive_inner(&mut self, cancel: &AtomicBool) -> Result<LinuxSupervisorTrust, ControlError> {
        if !self.receiving {
            return Err(ControlError::Protocol);
        }
        loop {
            self.check(cancel)?;
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
                match std::io::Error::last_os_error().kind() {
                    std::io::ErrorKind::Interrupted => continue,
                    std::io::ErrorKind::WouldBlock => {
                        self.pause();
                        continue;
                    }
                    _ => return Err(ControlError::Unconfirmed),
                }
            }
            // 在任何凭据/协议错误返回之前接管全部可见 FD；固定槽不因分配失败泄漏原生句柄。
            let mut owners: [Option<File>; 253] = std::array::from_fn(|_| None);
            let mut count = 0;
            let mut credentials = None;
            let mut valid = true;
            let mut header = unsafe { libc::CMSG_FIRSTHDR(&message) };
            while !header.is_null() {
                let base = unsafe { libc::CMSG_LEN(0) } as usize;
                let length = unsafe { (*header).cmsg_len };
                let offset = (header as usize).checked_sub(control.as_ptr() as usize);
                if length < base
                    || !offset.is_some_and(|offset| {
                        offset.checked_add(length).is_some_and(|end| {
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
                    if length % std::mem::size_of::<libc::c_int>() != 0 {
                        valid = false;
                    }
                    for index in 0..length / std::mem::size_of::<libc::c_int>() {
                        let fd = unsafe {
                            std::ptr::read_unaligned(data.cast::<libc::c_int>().add(index))
                        };
                        if fd < 0 {
                            valid = false;
                            continue;
                        }
                        // Linux 每包最多253个rights；接收缓冲更小。超额FD也立即接管并关闭。
                        let file = unsafe { File::from_raw_fd(fd) };
                        if count < owners.len() {
                            owners[count] = Some(file);
                            count += 1;
                        } else {
                            drop(file);
                            valid = false;
                        }
                    }
                } else if level == libc::SOL_SOCKET
                    && kind == libc::SCM_CREDENTIALS
                    && length == std::mem::size_of::<libc::ucred>()
                {
                    if credentials.is_some() {
                        valid = false;
                    }
                    credentials =
                        Some(unsafe { std::ptr::read_unaligned(data.cast::<libc::ucred>()) });
                } else {
                    valid = false;
                }
                header = unsafe { libc::CMSG_NXTHDR(&message, header) };
            }
            self.check(cancel)?;
            if !valid
                || received != 40
                || message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
                || count != 3
                || &bytes[..8] != MAGIC
                || bytes[8..40] != self.session
                || !credentials
                    .as_ref()
                    .is_some_and(|peer| self.peer.matches(peer))
            {
                return Err(ControlError::Protocol);
            }
            self.used = true;
            return Ok(LinuxSupervisorTrust::from_host(
                owners[0].take().expect("validated root"),
                owners[1].take().expect("validated user namespace"),
                owners[2].take().expect("validated mount namespace"),
            ));
        }
    }
}
