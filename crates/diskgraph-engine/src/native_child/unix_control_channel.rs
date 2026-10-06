use super::{ChildError, ControlWriteStatus};
use std::io;
use std::os::fd::AsRawFd;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::Stdio;

/// 独占 AF_UNIX 控制写端与一个有界未发送数据块，不修改宿主信号策略。
/// 来源：原生 Rust OS-child 的 socketpair、send 与描述符生命周期。
pub(super) struct UnixControlChannel {
    stream: Option<UnixStream>,
    pending: Option<Vec<u8>>,
}

impl UnixControlChannel {
    /// 接管原子 child 的父端 socket；调用前必须已有唯一 child owner。
    /// 参数：stream 是已持有的父端 AF_UNIX socket；返回：非阻塞单块写 owner 或原 I/O 错误。
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn from_stream(stream: UnixStream) -> Result<Self, ChildError> {
        close_on_exec(&stream)?;
        #[cfg(target_os = "macos")]
        suppress_sigpipe(&stream)?;
        stream
            .set_nonblocking(true)
            .map_err(|error| ChildError::io("nonblocking control socket", error))?;
        Ok(Self {
            stream: Some(stream),
            pending: None,
        })
    }

    /// 建立父非阻塞、子阻塞的 stdin 通道，两个端点都禁止额外 exec 继承。
    /// 参数：无；返回：父端唯一 owner 与交给 Command 的子 stdin，或原生能力/I/O 错误。
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn pair() -> Result<(Self, Stdio), ChildError> {
        let (parent, child) = UnixStream::pair()
            .map_err(|error| ChildError::io("create control socketpair", error))?;
        close_on_exec(&parent)?;
        close_on_exec(&child)?;
        parent
            .set_nonblocking(true)
            .map_err(|error| ChildError::io("nonblocking control socket", error))?;
        child
            .set_nonblocking(false)
            .map_err(|error| ChildError::io("blocking child stdin", error))?;
        #[cfg(target_os = "macos")]
        suppress_sigpipe(&parent)?;
        let child: OwnedFd = child.into();
        Ok((
            Self {
                stream: Some(parent),
                pending: None,
            },
            Stdio::from(child),
        ))
    }

    /// 拒绝尚未证明具有逐写 SIGPIPE 防护的平台，旧 Null 输入不受影响。
    /// 参数：无；返回：明确 Unsupported，不建立可能影响宿主信号的通道。
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(super) fn pair() -> Result<(Self, Stdio), ChildError> {
        Err(ChildError::Unsupported(
            "SIGPIPE-safe worker control is unavailable on this Unix platform",
        ))
    }

    /// 准入并复制一个新块，立即尝试一次原生发送；不在内部等待或重试。
    /// 参数：bytes 为非空且最多 4096 字节的块；返回：真实发送字节数或该块仍 Pending。
    pub(super) fn start_write(&mut self, bytes: &[u8]) -> Result<ControlWriteStatus, ChildError> {
        if self.stream.is_none() {
            return Err(ChildError::Unsupported("control input is closed"));
        }
        if self.pending.is_some() {
            return Err(ChildError::Unsupported("control write is already pending"));
        }
        if bytes.is_empty() || bytes.len() > ControlWriteStatus::MAX_CHUNK_BYTES {
            return Err(ChildError::Unsupported("invalid control chunk length"));
        }
        // 长度与状态在拥有前准入；Pending 后只保留这一份块，调用方不得覆盖。
        self.pending = Some(bytes.to_vec());
        self.send_once()
    }

    /// 对同一个 owned 块尝试一次原生发送，关闭后只报告 Closed。
    /// 参数：无；返回：真实写入/Pending/Closed，或空闲状态与实际 I/O 错误。
    pub(super) fn poll_write(&mut self) -> Result<ControlWriteStatus, ChildError> {
        if self.stream.is_none() {
            return Ok(ControlWriteStatus::Closed);
        }
        if self.pending.is_none() {
            return Err(ChildError::Unsupported("no pending control write"));
        }
        self.send_once()
    }

    /// 停止新的写入并舍弃尚未发送块；此方法不保证 raw 协议帧完整发送。
    /// 参数：无；返回：写端已关闭的 Closed；Unix 非阻塞 send 返回后没有借用缓冲区的在途 I/O。
    pub(super) fn close(&mut self) -> ControlWriteStatus {
        self.pending.take();
        self.stream.take();
        ControlWriteStatus::Closed
    }

    fn send_once(&mut self) -> Result<ControlWriteStatus, ChildError> {
        let stream = self
            .stream
            .as_ref()
            .ok_or(ChildError::Unsupported("control input is closed"))?;
        let bytes = self
            .pending
            .as_ref()
            .ok_or(ChildError::Unsupported("no pending control write"))?;
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_NOSIGNAL;
        #[cfg(not(target_os = "linux"))]
        let flags = 0;
        // 安全性：stream 独占有效 fd；bytes 在本次同步 send 返回前不移动或释放。
        // Linux 逐次禁止 SIGPIPE；Darwin 在建通道时对本 socket 设置 SO_NOSIGPIPE。
        let written = unsafe {
            libc::send(
                stream.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                flags,
            )
        };
        if written < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Ok(ControlWriteStatus::Pending);
            }
            return Err(ChildError::io("write child control input", error));
        }
        let written = written as usize;
        if written == 0 {
            return Err(ChildError::io(
                "write child control input",
                io::Error::new(io::ErrorKind::WriteZero, "control socket made no progress"),
            ));
        }
        if written > bytes.len() {
            return Err(ChildError::io(
                "write child control input",
                io::Error::new(io::ErrorKind::InvalidData, "invalid control write count"),
            ));
        }
        // 部分发送也只交付真实 n；调用方据 n 推进原帧，剩余字节不得隐式重发。
        self.pending.take();
        Ok(ControlWriteStatus::Written(written))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn close_on_exec(stream: &UnixStream) -> Result<(), ChildError> {
    let fd = stream.as_raw_fd();
    // 安全性：fd 在借用期间由本 stream 持有；只更改此描述符的 exec 继承标志。
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(ChildError::io(
            "close-on-exec control socket",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn suppress_sigpipe(stream: &UnixStream) -> Result<(), ChildError> {
    let enabled: libc::c_int = 1;
    // 安全性：选项只作用于 parent socket，值的指针与长度匹配 Darwin int ABI。
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&enabled as *const libc::c_int).cast(),
            std::mem::size_of_val(&enabled) as libc::socklen_t,
        )
    };
    if result < 0 {
        return Err(ChildError::io(
            "suppress control socket SIGPIPE",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}
