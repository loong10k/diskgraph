use super::original_client_output::OriginalClientOutput;
use super::{broker_frame, root_installation_path};
use crate::native_deadline::ClockStamp;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// 参数：socket为root保护的本地入口；返回：原监督角色的完整i32状态或明确协议/期限拒绝。
/// 固定doctor，无客户执行路径；原30秒涵盖连接、请求、全部输出与终态后的真实EOF。
pub fn run_client(socket: &Path) -> io::Result<i32> {
    run(socket, None)
}

/// 参数：固定socket和已解析只读查询；返回：原业务退出状态或有限协议/期限拒绝。
/// 不接受环境或执行路径，不绕过既有CLI/Engine资源授权。
pub fn run_query(socket: &Path, query: &super::BrokerQuery) -> io::Result<i32> {
    run(socket, Some(query))
}

fn run(socket: &Path, query: Option<&super::BrokerQuery>) -> io::Result<i32> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let (_parent, _ancestors) = root_installation_path::open_directory(
        socket
            .parent()
            .ok_or_else(|| io::Error::other("fixed socket parent missing"))?,
        deadline,
    )?;
    let metadata = std::fs::symlink_metadata(socket)?;
    if !metadata.file_type().is_socket() || metadata.uid() != 0 || metadata.nlink() != 1 {
        return Err(io::Error::other("untrusted broker socket"));
    }
    let mut stream = connect(socket, deadline)?;
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&peer) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of_val(&peer)
        || peer.uid != 0
        || peer.pid <= 0
    {
        return Err(io::Error::other("original root broker peer required"));
    }
    let expected =
        crate::recovery_slot::LinuxSupervisorPeer::from_host(peer.pid, peer.uid, peer.gid)
            .map_err(io::Error::other)?;
    let request = if let Some(query) = query {
        super::broker_query_request::BrokerQueryRequest::encode(query, deadline)?
    } else {
        let mut request = b"DG_FRONT_INPUT\n".to_vec();
        request.extend_from_slice(&serde_json::to_vec(
            &ClockStamp::capture(deadline).map_err(io::Error::other)?,
        )?);
        request
    };
    if request.len() > 4096 {
        return Err(io::Error::other("oversized fixed broker request"));
    }
    let mut offset = 0;
    while offset < request.len() {
        root_installation_path::check(deadline)?;
        match stream.write(&request[offset..]) {
            Ok(0) => return Err(io::Error::other("original broker write stopped")),
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                poll_until(stream.as_raw_fd(), libc::POLLOUT, deadline)?
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    stream.shutdown(std::net::Shutdown::Write)?;
    let output = OriginalClientOutput::new()?;
    let mut body = [0; 4096];
    let mut total = 0u64;
    loop {
        let mut header = [0; 5];
        read_exact(&mut stream, &mut header, &expected, deadline)?;
        let (kind, length) = broker_frame::decode(header)?;
        read_exact(&mut stream, &mut body[..length], &expected, deadline)?;
        total = total.saturating_add(length as u64);
        if total > 1 << 20 {
            return Err(io::Error::other("broker foreground output budget"));
        }
        if kind == 3 {
            let status = i32::from_be_bytes(body[..4].try_into().expect("validated exact status"));
            let mut extra = [0; 1];
            loop {
                root_installation_path::check(deadline)?;
                match receive(&stream, &mut extra, &expected, deadline) {
                    Ok(0) => return Ok(status),
                    Ok(_) => return Err(io::Error::other("broker data after terminal status")),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        poll_until(stream.as_raw_fd(), libc::POLLIN, deadline)?
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
        }
        output.write(kind, &body[..length], deadline)?;
    }
}

fn connect(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::other("invalid fixed broker socket length"));
    }
    address.sun_family = libc::AF_UNIX as _;
    for (to, from) in address.sun_path.iter_mut().zip(bytes) {
        *to = *from as libc::c_char;
    }
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(raw) };
    let enabled: libc::c_int = 1;
    if unsafe {
        libc::setsockopt(
            raw,
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            (&enabled as *const libc::c_int).cast(),
            std::mem::size_of_val(&enabled) as _,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    loop {
        root_installation_path::check(deadline)?;
        let result = unsafe {
            libc::connect(
                raw,
                (&address as *const libc::sockaddr_un).cast(),
                (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as _,
            )
        };
        if result == 0 {
            return Ok(stream);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EISCONN) => return Ok(stream),
            Some(libc::EINTR) => {}
            Some(libc::EAGAIN) => std::thread::sleep(Duration::from_millis(1)),
            Some(libc::EINPROGRESS) => {
                poll_until(raw, libc::POLLOUT, deadline)?;
                let mut pending = 0;
                let mut length = std::mem::size_of_val(&pending) as libc::socklen_t;
                if unsafe {
                    libc::getsockopt(
                        raw,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        (&mut pending as *mut libc::c_int).cast(),
                        &mut length,
                    )
                } != 0
                {
                    return Err(io::Error::last_os_error());
                }
                if pending != 0 {
                    return Err(io::Error::from_raw_os_error(pending));
                }
                return Ok(stream);
            }
            _ => return Err(error),
        }
    }
}

fn read_exact(
    stream: &mut UnixStream,
    bytes: &mut [u8],
    expected: &crate::recovery_slot::LinuxSupervisorPeer,
    deadline: Instant,
) -> io::Result<()> {
    let mut used = 0;
    while used < bytes.len() {
        root_installation_path::check(deadline)?;
        match receive(stream, &mut bytes[used..], expected, deadline) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated original broker frame",
                ));
            }
            Ok(count) => used += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                poll_until(stream.as_raw_fd(), libc::POLLIN, deadline)?
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// 参数：fd为原连接/输出，events为读写就绪，deadline为原期限；返回：就绪或期限/OS拒绝。
/// HUP只让原read/write确认实际EOF/错误，不当作完整帧或成功终态。
pub(super) fn poll_until(
    fd: libc::c_int,
    events: libc::c_short,
    deadline: Instant,
) -> io::Result<()> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "original frontend deadline"))?;
        let millis = remaining.as_millis().min(i32::MAX as u128) as i32;
        if millis <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "original frontend deadline",
            ));
        }
        let mut ready = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut ready, 1, millis) };
        if result > 0 {
            if ready.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::other("invalid original frontend descriptor"));
            }
            return Ok(());
        }
        if result == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "original frontend deadline",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn receive(
    stream: &UnixStream,
    bytes: &mut [u8],
    expected: &crate::recovery_slot::LinuxSupervisorPeer,
    deadline: Instant,
) -> io::Result<usize> {
    root_installation_path::check(deadline)?;
    let mut control = [0u64; 64];
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
            stream.as_raw_fd(),
            &mut message,
            libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
        )
    };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    // 先处置所有实际交付的辅助FD，再检查取消/期限和原root发送者；SO_PEERCRED不能替代逐消息身份。
    let actual = crate::recovery_control::linux_message_credentials::inspect_credentials(
        &message,
        control.as_ptr() as usize,
        std::mem::size_of_val(&control),
    )
    .map_err(io::Error::other)?;
    root_installation_path::check(deadline)?;
    if count > 0
        && !actual
            .as_ref()
            .is_some_and(|actual| expected.matches(actual))
    {
        return Err(io::Error::other("original root broker sender changed"));
    }
    Ok(count as usize)
}
