use super::guardian_frontend_binding::GuardianFrontendBinding;
use super::{linux_broker_client, root_installation_path};
use crate::recovery_slot::LinuxSupervisorPeer;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

/// 原root握手的固定八字节协议标记，不是客户自报身份或业务结果。
pub(super) const ROOT_HELLO: &[u8; 8] = b"DGGU01R\n";

/// 参数：原受保护socket路径/期限；返回：公开安装绑定或旧模式，损坏绑定禁止降级。
pub(super) fn installed_binding(
    socket: &Path,
    deadline: Instant,
) -> io::Result<Option<GuardianFrontendBinding>> {
    let path = socket
        .parent()
        .ok_or_else(|| io::Error::other("missing socket parent"))?
        .join("guardian-frontend.json");
    let (mut original, _ancestors) = match root_installation_path::open_public(&path, deadline) {
        Ok(opened) => opened,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let before = original.metadata()?;
    if before.len() == 0 || before.len() > 4096 {
        return Err(io::Error::other("invalid guardian binding size"));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    // 固定最大4097字节观察增长；始终沿同一File读取，不重新解析公开路径。
    original.by_ref().take(4097).read_to_end(&mut bytes)?;
    root_installation_path::check(deadline)?;
    if root_installation_path::identity(&before)
        != root_installation_path::identity(&original.metadata()?)
    {
        return Err(io::Error::other("original guardian binding changed"));
    }
    GuardianFrontendBinding::decode(
        &bytes,
        socket
            .to_str()
            .ok_or_else(|| io::Error::other("invalid socket path"))?,
    )
    .map(Some)
}

/// 参数：原连接、受信安装身份与原期限；返回：由原root握手绑定的同PID永久guardian凭据。
/// 不采用监听者PID，不接受非root自报身份；公开FD不转交executor，凭据始终来自recvmsg。
pub(super) fn authenticate(
    stream: &mut UnixStream,
    binding: &GuardianFrontendBinding,
    deadline: Instant,
) -> io::Result<LinuxSupervisorPeer> {
    let actual = peek_root_sender(stream, deadline)?;
    let root = LinuxSupervisorPeer::from_host(actual.pid, actual.uid, actual.gid)
        .map_err(io::Error::other)?;
    let mut hello = [0; 8];
    linux_broker_client::read_exact(stream, &mut hello, &root, deadline)?;
    if &hello != ROOT_HELLO {
        return Err(io::Error::other("invalid original root guardian greeting"));
    }
    let (uid, gid) = binding.guardian_identity();
    LinuxSupervisorPeer::from_host(actual.pid, uid, gid).map_err(io::Error::other)
}

fn peek_root_sender(stream: &UnixStream, deadline: Instant) -> io::Result<libc::ucred> {
    loop {
        root_installation_path::check(deadline)?;
        let mut byte = [0u8; 1];
        let mut control = [0u64; 64];
        let mut vector = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: 1,
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = std::mem::size_of_val(&control);
        let count = unsafe {
            libc::recvmsg(
                stream.as_raw_fd(),
                &mut message,
                libc::MSG_PEEK | libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                linux_broker_client::poll_until(stream.as_raw_fd(), libc::POLLIN, deadline)?;
                continue;
            }
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        // 即使peek也先关闭/拒绝实际交付的辅助FD；不能遗漏peek复制出来的描述符。
        let credentials = crate::recovery_control::linux_message_credentials::inspect_credentials(
            &message,
            control.as_ptr() as usize,
            std::mem::size_of_val(&control),
        )
        .map_err(io::Error::other)?;
        root_installation_path::check(deadline)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "missing original root guardian greeting",
            ));
        }
        let actual = credentials
            .ok_or_else(|| io::Error::other("missing original root guardian credentials"))?;
        if actual.uid != 0 || actual.pid <= 0 {
            return Err(io::Error::other("root guardian greeting required"));
        }
        return Ok(actual);
    }
}
