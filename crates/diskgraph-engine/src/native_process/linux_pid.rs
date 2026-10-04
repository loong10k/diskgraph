use super::ProcessNativeSession;
use super::linux_directory::LinuxDirectory;
use super::linux_open::{filesystem, last_error, numeric_name, open_at};
use super::linux_target::LinuxTarget;
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::fs::{File, Metadata};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;

/// held PID 目录与前后启动时间；来源：Linux proc_pid_stat(5)，旧目录不会追踪重用后的 PID。
pub(super) struct LinuxPid {
    file: File,
    pub(super) start_ticks: u64,
    own_process: bool,
}
impl LinuxPid {
    /// 参数：实际 procfs 根、数字 PID、自进程目录身份、同账本；返回：绑定启动对象的租约。
    pub(super) fn open(
        root: &File,
        pid: u32,
        own: &Metadata,
        session: &ProcessNativeSession<'_>,
    ) -> Result<Self, Failure> {
        session.admit(512, 1, 0)?;
        let mut name_buffer = [0_u8; 12];
        let name = numeric_name(pid, &mut name_buffer);
        let file = open_at(
            root.as_raw_fd(),
            name,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x08 | 0x04 | 0x02 | 0x01,
        )?;
        session.check()?;
        let metadata = file.metadata().map_err(|_| last_error())?;
        let start_ticks = read_start(&file, pid, session)?;
        Ok(Self {
            file,
            start_ticks,
            own_process: metadata.dev() == own.dev() && metadata.ino() == own.ino(),
        })
    }
    /// 参数：本资源 held target、PID 和原账本；返回：逐资源真实匹配，前后启动身份变化拒绝。
    pub(super) fn matches(
        &self,
        target: &LinuxTarget<'_>,
        pid: u32,
        session: &ProcessNativeSession<'_>,
    ) -> Result<bool, Failure> {
        session.check()?;
        let fd_root = open_at(
            self.file.as_raw_fd(),
            c"fd",
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x08 | 0x04 | 0x02 | 0x01,
        )?;
        let anchor = fd_root.try_clone().map_err(|_| last_error())?;
        let mut entries = LinuxDirectory::new(fd_root, session)?;
        let mut matched = false;
        while let Some(fd) = entries.next_number(session)? {
            // 只排除本次目标 O_PATH 句柄；同 PID 的真实其他占用必须保留。
            if self.own_process && i64::from(fd) == i64::from(target.file.as_raw_fd()) {
                continue;
            }
            session.admit(1024, 1, 0)?;
            let mut name_buffer = [0_u8; 12];
            let name = numeric_name(fd, &mut name_buffer);
            // procfs 的 FD magic link 是本方法唯一有意跟随的内核关联；不读取目标正文。
            let file = match open_at(anchor.as_raw_fd(), name, libc::O_PATH | libc::O_CLOEXEC, 0) {
                Ok(file) => file,
                Err(Failure::Conflict) => continue,
                Err(error) => return Err(error),
            };
            session.check()?;
            if !matches!(filesystem(&file)?, 0xef53 | 0x01021994) {
                continue;
            }
            session.check()?;
            let metadata = file.metadata().map_err(|_| last_error())?;
            session.check()?;
            if metadata.is_file()
                && metadata.dev() == target.device
                && metadata.ino() == target.inode
            {
                matched = true;
            }
        }
        if read_start(&self.file, pid, session)? != self.start_ticks {
            return Err(Failure::Conflict);
        }
        Ok(matched)
    }
}

fn read_start(
    directory: &File,
    pid: u32,
    session: &ProcessNativeSession<'_>,
) -> Result<u64, Failure> {
    session.admit(4096, 1, 0)?;
    let mut file = open_at(
        directory.as_raw_fd(),
        c"stat",
        libc::O_RDONLY | libc::O_CLOEXEC,
        0x08 | 0x04 | 0x02 | 0x01,
    )?;
    let mut raw = [0_u8; 4096];
    let length = file.read(&mut raw).map_err(|_| last_error())?;
    session.check()?;
    if length == raw.len() {
        return Err(Failure::BudgetExceeded);
    }
    let bytes = &raw[..length];
    let split = bytes
        .iter()
        .rposition(|b| *b == b')')
        .ok_or(Failure::Unavailable)?;
    let prefix = bytes
        .iter()
        .position(|b| *b == b' ')
        .ok_or(Failure::Unavailable)?;
    if parse_u64(&bytes[..prefix])? != u64::from(pid) {
        return Err(Failure::Conflict);
    }
    let value = bytes[split + 1..]
        .split(u8::is_ascii_whitespace)
        .filter(|v| !v.is_empty())
        .nth(19)
        .ok_or(Failure::Unavailable)?;
    let ticks = parse_u64(value)?;
    if ticks == 0 {
        return Err(Failure::Unsupported);
    }
    Ok(ticks)
}
fn parse_u64(raw: &[u8]) -> Result<u64, Failure> {
    if raw.is_empty() {
        return Err(Failure::Unavailable);
    }
    raw.iter().try_fold(0_u64, |value, b| {
        if b.is_ascii_digit() {
            value
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(b - b'0')))
                .ok_or(Failure::Unavailable)
        } else {
            Err(Failure::Unavailable)
        }
    })
}
