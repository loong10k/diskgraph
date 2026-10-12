use super::{
    broker_frame, linux_broker_client, original_client_input::OriginalClientInput,
    original_client_output::OriginalClientOutput,
};
use crate::recovery_slot::LinuxSupervisorPeer;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// 参数：原已完成root握手的stream和同一guardian身份；返回：原MCP终态后的真实EOF。
/// 输入/输出使用固定缓冲，每笔socket输出继续认证，原stdin/stdout flags退出还原。
pub(super) fn relay(mut stream: UnixStream, expected: LinuxSupervisorPeer) -> io::Result<i32> {
    let input = OriginalClientInput::new()?;
    let output = OriginalClientOutput::new()?;
    let mut input_ended = false;
    let mut bytes = [0u8; 4096];
    let mut total = 0u64;
    loop {
        if !input_ended {
            match input.read(&mut bytes) {
                Ok(0) => {
                    stream.shutdown(std::net::Shutdown::Write)?;
                    input_ended = true;
                }
                Ok(count) => {
                    charge(&mut total, count)?;
                    let deadline = Instant::now() + Duration::from_secs(30);
                    let mut sent = 0;
                    while sent < count {
                        super::root_installation_path::check(deadline)?;
                        match stream.write(&bytes[sent..count]) {
                            Ok(0) => return Err(io::Error::other("original MCP input stopped")),
                            Ok(n) => sent += n,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                linux_broker_client::poll_until(
                                    stream.as_raw_fd(),
                                    libc::POLLOUT,
                                    deadline,
                                )?
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut header = [0u8; 5];
        match linux_broker_client::receive(&stream, &mut header[..1], &expected, deadline) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "MCP session missing terminal status",
                ));
            }
            Ok(_) => {
                // 首字节到达才启动原单帧期限，空闲会话不冒用启动30秒作为总寿命。
                linux_broker_client::read_exact(
                    &mut stream,
                    &mut header[1..],
                    &expected,
                    deadline,
                )?;
                let (kind, length) = broker_frame::decode(header)?;
                linux_broker_client::read_exact(
                    &mut stream,
                    &mut bytes[..length],
                    &expected,
                    deadline,
                )?;
                charge(&mut total, length)?;
                if kind == 3 {
                    let status =
                        i32::from_be_bytes(bytes[..4].try_into().expect("validated status"));
                    let mut extra = [0u8; 1];
                    loop {
                        match linux_broker_client::receive(&stream, &mut extra, &expected, deadline)
                        {
                            Ok(0) => return Ok(status),
                            Ok(_) => {
                                return Err(io::Error::other("MCP bytes after terminal status"));
                            }
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                linux_broker_client::poll_until(
                                    stream.as_raw_fd(),
                                    libc::POLLIN,
                                    deadline,
                                )?
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
                output.write(kind, &bytes[..length], deadline)?;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        let mut descriptors = [
            libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: if input_ended { -1 } else { libc::STDIN_FILENO },
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(descriptors.as_mut_ptr(), 2, 20) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        if descriptors
            .iter()
            .any(|fd| fd.revents & libc::POLLNVAL != 0)
        {
            return Err(io::Error::other("invalid original MCP descriptor"));
        }
    }
}

fn charge(total: &mut u64, count: usize) -> io::Result<()> {
    *total = total
        .checked_add(count as u64)
        .ok_or_else(|| io::Error::other("MCP session overflow"))?;
    if *total > 2 << 30 {
        return Err(io::Error::other("MCP session stream budget"));
    }
    Ok(())
}
