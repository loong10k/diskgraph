use crate::EngineError;
use crate::recovery_control::UnixControlWriter;
use diskgraph_core::BusinessError;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::time::Instant;

/// 采用原 root 出生 JSON 绑定的独占私有控制 FD；来源：Linux 内核对象／PF-06。
/// 参数：fd/birth_fd为不同原描述符，parent_pid/session绑定已认证原父及摘要，deadline不刷新。
/// lifecycle仅来自原认证schema3；选择通知版本2，不能在旧版本超时后改变。
/// 返回：同出生 root 创建的未命名 AF_UNIX stream writer；不能从环境值或对端声明自认证。
pub(crate) fn adopt_linux_supervisor_control(
    fd: u32,
    birth_fd: libc::c_int,
    parent_pid: libc::pid_t,
    session: [u8; 32],
    deadline: Instant,
    lifecycle: bool,
) -> Result<UnixControlWriter, EngineError> {
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    let fd = libc::c_int::try_from(fd).map_err(|_| BusinessError::PermissionDenied)?;
    if fd < 3 || fd == birth_fd || unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(BusinessError::PermissionDenied.into());
    }
    for (option, expected) in [
        (libc::SO_TYPE, libc::SOCK_STREAM),
        (libc::SO_DOMAIN, libc::AF_UNIX),
    ] {
        let mut actual: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&actual) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                option,
                (&mut actual as *mut libc::c_int).cast(),
                &mut length,
            )
        } != 0
            || length as usize != std::mem::size_of_val(&actual)
            || actual != expected
        {
            return Err(BusinessError::PermissionDenied.into());
        }
    }
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&peer) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of_val(&peer)
        || peer.pid != parent_pid
        || peer.uid != 0
        || peer.gid != 0
    {
        return Err(BusinessError::PermissionDenied.into());
    }
    // 此处不是用创建者凭据认证后续发送者；原父已在材料 datagram 中通过逐消息内核凭据认证。
    // 控制通道必须由该原父创建且由已认证 JSON 指定，broker 还须验证原服务的逐消息凭据。
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let stream = UnixStream::from(owned);
    if !stream.local_addr()?.is_unnamed() || !stream.peer_addr()?.is_unnamed() {
        return Err(BusinessError::PermissionDenied.into());
    }
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(BusinessError::PermissionDenied.into());
    }
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    let writer = if lifecycle {
        UnixControlWriter::new_lifecycle(stream, session)
    } else {
        UnixControlWriter::new(stream, session, deadline)
    };
    writer.map_err(|_| BusinessError::PermissionDenied.into())
}
