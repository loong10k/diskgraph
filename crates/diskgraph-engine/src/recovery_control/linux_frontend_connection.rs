use super::{ControlError, FrontendRead, UnixForegroundOutput};
use crate::recovery_slot::LinuxSupervisorPeer;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// 公共Linux前端原连接的逐消息身份与有界输入owner；来源：PF-06本地broker认证合同。
/// 固定UID来自受信部署配置；连接创建者身份不代替实际发送者，不授予Engine或远程工具权限。
pub struct LinuxFrontendConnection {
    output: UnixForegroundOutput,
    peer: LinuxSupervisorPeer,
    bytes: [u8; 4096],
    failure: Option<ControlError>,
    ended: bool,
}

impl LinuxFrontendConnection {
    /// 参数：listener为宿主已绑定的原监听对象；返回：接收前启用逐消息凭据或原错误。
    /// 必须先于accept/客户发送；不认证socket路径、安装者、镜像或准入客户。
    pub fn prepare_listener(listener: &UnixListener) -> Result<(), ControlError> {
        let enabled: libc::c_int = 1;
        if unsafe {
            libc::setsockopt(
                listener.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as _,
            )
        } != 0
        {
            return Err(ControlError::Unconfirmed);
        }
        Ok(())
    }

    /// 参数：output为accept后的原公共连接，expected_uid为可信配置中的非root客户UID；返回：原身份绑定或拒绝。
    /// 接收前须已启用SO_PASSCRED；不接受客户端自报UID，不从环境或任意路径推导安装信任。
    pub fn new(output: UnixForegroundOutput, expected_uid: u32) -> Result<Self, ControlError> {
        let fd = output.raw_fd().ok_or(ControlError::Protocol)?;
        if expected_uid == 0 || expected_uid == u32::MAX {
            return Err(ControlError::Protocol);
        }
        let mut actual: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&actual) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut actual as *mut libc::ucred).cast(),
                &mut length,
            )
        } != 0
            || length as usize != std::mem::size_of_val(&actual)
            || actual.uid != expected_uid
        {
            return Err(ControlError::Protocol);
        }
        let mut enabled: libc::c_int = 0;
        length = std::mem::size_of_val(&enabled) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
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
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(ControlError::Unconfirmed);
        }
        Ok(Self {
            output,
            peer: LinuxSupervisorPeer::from_host(actual.pid, actual.uid, actual.gid)?,
            bytes: [0; 4096],
            failure: None,
            ended: false,
        })
    }

    fn check(&self, deadline: Instant, cancel: &AtomicBool) -> Result<(), ControlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(ControlError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(ControlError::Deadline);
        }
        Ok(())
    }

    /// 参数：deadline/cancel为原单次输入预算；返回：最多4096字节借用、Pending或输入EOF，错误永久锁存。
    /// 每次仅一笔recvmsg；先关闭附加FD，再核对原发送者。输入EOF仍允许原输出完成。
    pub fn poll_input_until(
        &mut self,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<FrontendRead<'_>, ControlError> {
        let received = self.receive_once(deadline, cancel);
        match received {
            Ok(Some(count)) => Ok(FrontendRead::Data(&self.bytes[..count])),
            Ok(None) if self.ended => Ok(FrontendRead::Eof),
            Ok(None) => Ok(FrontendRead::Pending),
            Err(error) => {
                self.failure = Some(error);
                let _ = self.output.finish();
                Err(error)
            }
        }
    }

    fn receive_once(
        &mut self,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<Option<usize>, ControlError> {
        self.check(deadline, cancel)?;
        if self.ended {
            return Ok(None);
        }
        let fd = self.output.raw_fd().ok_or(ControlError::Protocol)?;
        let mut control = [0_u64; 64];
        let mut vector = libc::iovec {
            iov_base: self.bytes.as_mut_ptr().cast(),
            iov_len: self.bytes.len(),
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = std::mem::size_of_val(&control);
        let count = unsafe {
            libc::recvmsg(
                fd,
                &mut message,
                libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
            )
        };
        if count < 0 {
            self.check(deadline, cancel)?;
            return match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => Ok(None),
                _ => Err(ControlError::Unconfirmed),
            };
        }
        let credentials = super::linux_message_credentials::inspect_credentials(
            &message,
            control.as_ptr() as usize,
            std::mem::size_of_val(&control),
        );
        self.check(deadline, cancel)?;
        let credentials = credentials?;
        if count == 0 {
            self.ended = true;
            return Ok(None);
        }
        if count as usize > self.bytes.len()
            || !credentials
                .as_ref()
                .is_some_and(|actual| self.peer.matches(actual))
        {
            return Err(ControlError::Protocol);
        }
        Ok(Some(count as usize))
    }

    /// 参数：bytes/deadline/cancel为原公共输出块及交付预算；返回：完整交付或原错误，不刷新授权。
    pub fn write_output_until(
        &mut self,
        bytes: &[u8],
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> std::io::Result<()> {
        self.output.write_chunk_until(bytes, deadline, cancel)
    }

    /// 参数：无；返回：消费同一连接的输出owner；调用方须停止输入，用于绑定原监督进程的输出。
    /// 不克隆原FD、不认定恢复完成、不授予新身份或命令权限。
    pub fn into_output(self) -> Result<UnixForegroundOutput, ControlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        Ok(self.output)
    }
}
