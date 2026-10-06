use super::ChildError;
use super::child_read_buffer::ChildReadBuffer;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

/// 独占父端管道及出生前准备的五个高位 child fd，不维护第二个进程 owner。
/// 来源：原生 Rust pipe2/socketpair/F_DUPFD_CLOEXEC 与固定描述符 allowlist。
pub(super) struct LinuxAtomicPipes {
    pub(super) buffer: ChildReadBuffer,
    pub(super) control: Option<UnixStream>,
    pub(super) stdout: Option<File>,
    pub(super) stderr: Option<File>,
    pub(super) startup: Option<OwnedFd>,
    child_ends: Option<[OwnedFd; 5]>,
}

impl LinuxAtomicPipes {
    /// 参数：image 为宿主已持有的 ELF；返回：全部 CLOEXEC 的准备材料与高位源 fd。
    pub(super) fn new(image: &File) -> Result<Self, ChildError> {
        let buffer = ChildReadBuffer::new()?;
        let (control, input) = UnixStream::pair()
            .map_err(|error| ChildError::io("atomic control socketpair", error))?;
        let (stdout, output) = pipe()?;
        let (stderr, errors) = pipe()?;
        let mut sockets = [-1; 2];
        if unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                0,
                sockets.as_mut_ptr(),
            )
        } < 0
        {
            return Err(ChildError::io(
                "atomic startup socketpair",
                io::Error::last_os_error(),
            ));
        }
        let startup = unsafe { OwnedFd::from_raw_fd(sockets[0]) };
        let child_startup = unsafe { OwnedFd::from_raw_fd(sockets[1]) };
        let child_ends = [
            duplicate(input.as_raw_fd())?,
            duplicate(output.as_raw_fd())?,
            duplicate(errors.as_raw_fd())?,
            duplicate(image.as_raw_fd())?,
            duplicate(child_startup.as_raw_fd())?,
        ];
        Ok(Self {
            buffer,
            control: Some(control),
            stdout: Some(stdout),
            stderr: Some(stderr),
            startup: Some(startup),
            child_ends: Some(child_ends),
        })
    }

    /// 参数：无；返回：固定槽 0..4 对应的五个源 fd，只供准备好的 native child 借用。
    pub(super) fn child_fds(&self) -> [i32; 5] {
        let ends = self
            .child_ends
            .as_ref()
            .expect("prepared child descriptor ownership");
        std::array::from_fn(|index| ends[index].as_raw_fd())
    }

    /// 参数：无；返回：释放父侧临时 child 端，只有真实 child 保留自己的 COW fd 表。
    pub(super) fn close_child_copies(&mut self) {
        self.child_ends.take();
    }

    /// 参数：无；返回：原 fd 非阻塞设置结果；调用点必须在原子 owner 已存在之后。
    pub(super) fn configure(&self) -> Result<(), ChildError> {
        for raw in [
            self.stdout.as_ref().map(AsRawFd::as_raw_fd),
            self.stderr.as_ref().map(AsRawFd::as_raw_fd),
            self.startup.as_ref().map(AsRawFd::as_raw_fd),
        ] {
            let raw = raw.ok_or(ChildError::Unsupported("atomic pipe ownership missing"))?;
            let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(ChildError::io(
                    "nonblocking atomic child pipe",
                    io::Error::last_os_error(),
                ));
            }
        }
        Ok(())
    }
}

fn duplicate(fd: i32) -> Result<OwnedFd, ChildError> {
    let raw = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
    if raw < 0 {
        return Err(ChildError::io(
            "prepare atomic child descriptor",
            io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn pipe() -> Result<(File, OwnedFd), ChildError> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(ChildError::io(
            "atomic output pipe",
            io::Error::last_os_error(),
        ));
    }
    Ok((unsafe { File::from_raw_fd(fds[0]) }, unsafe {
        OwnedFd::from_raw_fd(fds[1])
    }))
}
