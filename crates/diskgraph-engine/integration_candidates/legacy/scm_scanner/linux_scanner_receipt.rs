use super::ChildError;
use super::linux_scanner_exit::LinuxScannerExit;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// 私有启动通道接收原 child 自身 pidfd，避免父侧按数字 PID 再打开身份。
/// 来源：Linux AF_UNIX SCM_RIGHTS、MSG_CMSG_CLOEXEC 与 retained direct-child 资格。
pub(super) struct LinuxScannerReceipt {
    receiver: OwnedFd,
}

impl LinuxScannerReceipt {
    /// 建立一条固定消息通道。参数：无；返回：父 receiver/child sender，均非阻塞且 CLOEXEC。
    pub(super) fn pair() -> Result<(Self, OwnedFd), ChildError> {
        let mut fds = [-1; 2];
        if unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                0,
                fds.as_mut_ptr(),
            )
        } < 0
        {
            return Err(ChildError::io(
                "scanner pidfd channel",
                io::Error::last_os_error(),
            ));
        }
        // 安全性：socketpair 成功交付两个不同、独占的新 fd。
        Ok(unsafe {
            (
                Self {
                    receiver: OwnedFd::from_raw_fd(fds[0]),
                },
                OwnedFd::from_raw_fd(fds[1]),
            )
        })
    }

    /// 查询 child 中需要关闭的父端副本。参数：无；返回：仅 pre_exec 使用的借用 fd。
    pub(super) fn receiver_fd(&self) -> RawFd {
        self.receiver.as_raw_fd()
    }

    /// pre_exec 发送自身 pidfd；参数：sender/pid 为固定 child 通道与真实 getpid。
    /// 返回：原 syscall 错误；发送不等待队列空间，ACK 按 20ms syscall 轮询等父决定，不分配 Rust 对象。
    pub(super) unsafe fn send_self(sender: RawFd, pid: libc::pid_t) -> io::Result<()> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = fd as RawFd;
        let mut pid_bytes = pid.to_ne_bytes();
        let mut iov = libc::iovec {
            iov_base: pid_bytes.as_mut_ptr().cast(),
            iov_len: pid_bytes.len(),
        };
        let mut ancillary = [0usize; 8];
        let length = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
        let header = ancillary.as_mut_ptr().cast::<libc::cmsghdr>();
        unsafe {
            (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as usize;
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            libc::CMSG_DATA(header).cast::<RawFd>().write(fd);
        }
        let message = libc::msghdr {
            msg_name: std::ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: ancillary.as_mut_ptr().cast(),
            msg_controllen: length,
            msg_flags: 0,
        };
        let sent =
            unsafe { libc::sendmsg(sender, &message, libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) };
        let result = if sent < 0 {
            Err(io::Error::last_os_error())
        } else if sent != pid_bytes.len() as isize {
            Err(io::Error::from_raw_os_error(libc::EIO))
        } else {
            Ok(())
        };
        // Linux close 即使 EINTR 也不重用这个数字描述符；父已有或队列持有唯一 file 引用。
        unsafe {
            libc::close(fd);
        }
        result?;
        // 父只有持有/核验本 pidfd 后才能允许 exec。父错误或 unwind 关闭通道，
        // child 返回 ECANCELED，std spawn 的失败路径真实回收，不能留下已执行 scanner。
        let mut acknowledgement = [0u8; 1];
        loop {
            let received = unsafe {
                libc::recv(
                    sender,
                    acknowledgement.as_mut_ptr().cast(),
                    1,
                    libc::MSG_DONTWAIT,
                )
            };
            if received == 1 {
                return if acknowledgement[0] == 1 {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(libc::EPROTO))
                };
            }
            if received == 0 {
                return Err(io::Error::from_raw_os_error(libc::ECANCELED));
            }
            let error = io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(libc::EAGAIN | libc::EINTR)) {
                return Err(error);
            }
            let mut view = libc::pollfd {
                fd: sender,
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut view, 1, 20) } < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EINTR) {
                    return Err(error);
                }
            }
        }
    }

    /// exec 前接收一次固定消息。参数：无；返回：内核核验的原 direct-child pidfd 资格。
    /// 所有收到的 ancillary fd 在错误路径自动关闭；未知/截断内容不授予正常许可。
    pub(super) fn receive(&self) -> Result<Option<LinuxScannerExit>, ChildError> {
        let mut bytes = [0u8; 4];
        let mut ancillary = [0usize; 8];
        let mut iov = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut message = libc::msghdr {
            msg_name: std::ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: ancillary.as_mut_ptr().cast(),
            msg_controllen: std::mem::size_of_val(&ancillary),
            msg_flags: 0,
        };
        let received = unsafe {
            libc::recvmsg(
                self.receiver.as_raw_fd(),
                &mut message,
                libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
            )
        };
        if received < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Ok(None);
            }
            return Err(ChildError::io("receive scanner pidfd", error));
        }
        if received == 0 {
            return Err(ChildError::Unsupported(
                "scanner pidfd channel closed before receipt",
            ));
        }
        let mut descriptors: [Option<OwnedFd>; 12] = std::array::from_fn(|_| None);
        let mut count = 0;
        let mut malformed = message.msg_controllen > std::mem::size_of_val(&ancillary);
        let capacity = message
            .msg_controllen
            .min(std::mem::size_of_val(&ancillary));
        let header_size = unsafe { libc::CMSG_LEN(0) } as usize;
        let base = ancillary.as_ptr().cast::<u8>();
        let mut offset = 0;
        while offset + header_size <= capacity {
            let header = unsafe { base.add(offset).cast::<libc::cmsghdr>().read_unaligned() };
            if header.cmsg_len < header_size || header.cmsg_len > capacity - offset {
                malformed = true;
                break;
            }
            let payload = header.cmsg_len - header_size;
            if header.cmsg_level != libc::SOL_SOCKET || header.cmsg_type != libc::SCM_RIGHTS {
                malformed = true;
            } else {
                malformed |= payload % std::mem::size_of::<RawFd>() != 0;
                for index in 0..payload / std::mem::size_of::<RawFd>() {
                    let raw = unsafe {
                        base.add(offset + header_size + index * std::mem::size_of::<RawFd>())
                            .cast::<RawFd>()
                            .read_unaligned()
                    };
                    if raw < 0 {
                        malformed = true;
                        continue;
                    }
                    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
                    if count < descriptors.len() {
                        descriptors[count] = Some(fd);
                        count += 1;
                    } else {
                        malformed = true;
                        drop(fd);
                    }
                }
            }
            let alignment = std::mem::size_of::<usize>();
            offset += (header.cmsg_len + alignment - 1) & !(alignment - 1);
        }
        if malformed
            || received != 4
            || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
            || count != 1
            || i32::from_ne_bytes(bytes) <= 0
        {
            return Err(ChildError::Unsupported(
                "scanner pidfd receipt is malformed or truncated",
            ));
        }
        let fd = descriptors[0].take().ok_or(ChildError::Unsupported(
            "scanner pidfd receipt has no descriptor",
        ))?;
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
        if flags < 0 {
            return Err(ChildError::io(
                "inspect scanner pidfd",
                io::Error::last_os_error(),
            ));
        }
        if flags & libc::FD_CLOEXEC == 0 {
            return Err(ChildError::Unsupported("scanner pidfd is inheritable"));
        }
        LinuxScannerExit::new(fd, i32::from_ne_bytes(bytes) as u32).map(Some)
    }

    /// 仅资格核验成功后允许子进程 exec。参数：无；返回：一次实际非阻塞 ACK 的原 I/O 结果。
    pub(super) fn allow_exec(&self) -> Result<(), ChildError> {
        let byte = [1u8];
        let sent = unsafe {
            libc::send(
                self.receiver.as_raw_fd(),
                byte.as_ptr().cast(),
                1,
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if sent < 0 {
            return Err(ChildError::io(
                "acknowledge scanner pidfd",
                io::Error::last_os_error(),
            ));
        }
        if sent != 1 {
            return Err(ChildError::Unsupported(
                "scanner pidfd acknowledgement was partial",
            ));
        }
        Ok(())
    }

    /// 等待一轮通道可读，不创建新请求期限。参数：无；返回：可读/挂断或 20ms 暂无消息。
    pub(super) fn wait_readable(&self) -> Result<bool, ChildError> {
        let mut view = libc::pollfd {
            fd: self.receiver.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut view, 1, 20) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(ChildError::io("wait scanner receipt", error));
        }
        if view.revents & libc::POLLNVAL != 0 {
            return Err(ChildError::Unsupported("scanner receipt descriptor lost"));
        }
        Ok(view.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
    }

    /// 拒绝尚未允许的 exec，同时保留 fd 数字直到 launcher join。
    /// 参数：无；返回：真实 shutdown 结果；child 收到 EOF，不依赖 Rust 析构或数值 PID。
    pub(super) fn cancel(&self) -> Result<(), ChildError> {
        if unsafe { libc::shutdown(self.receiver.as_raw_fd(), libc::SHUT_RDWR) } < 0 {
            return Err(ChildError::io(
                "close scanner acknowledgement channel",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}
