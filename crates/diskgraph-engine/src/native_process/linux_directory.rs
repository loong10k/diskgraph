use super::ProcessNativeSession;
use super::linux_open::last_error;
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::fs::File;
use std::os::fd::AsRawFd;

/// 固定栈缓冲逐条读取 held procfs 目录；来源：Linux getdents64(2) 的原生记录布局。
/// 不分配全机目录列表，也不让 libc 目录流分配绕过本会话容量。
pub(super) struct LinuxDirectory {
    file: File,
    bytes: [u8; 4096],
    position: usize,
    length: usize,
}
impl LinuxDirectory {
    /// 参数：已经过路径约束的目录句柄与账本；返回：独占枚举租约。
    pub(super) fn new(file: File, session: &ProcessNativeSession<'_>) -> Result<Self, Failure> {
        session.check()?;
        Ok(Self {
            file,
            bytes: [0; 4096],
            position: 0,
            length: 0,
        })
    }
    /// 参数：同账本；返回：下一真实十进制 PID/FD，零可作为 FD，不保留程序名称。
    pub(super) fn next_number(
        &mut self,
        session: &ProcessNativeSession<'_>,
    ) -> Result<Option<u32>, Failure> {
        loop {
            if self.position == self.length {
                session.admit(self.bytes.len() as u64, 0, 0)?;
                let length = unsafe {
                    libc::syscall(
                        libc::SYS_getdents64,
                        self.file.as_raw_fd(),
                        self.bytes.as_mut_ptr(),
                        self.bytes.len(),
                    )
                };
                session.check()?;
                if length < 0 {
                    return Err(last_error());
                }
                if length == 0 {
                    return Ok(None);
                }
                self.length = usize::try_from(length).map_err(|_| Failure::InternalError)?;
                self.position = 0;
                if self.length > self.bytes.len() {
                    return Err(Failure::InternalError);
                }
            }
            session.admit(0, 1, 0)?;
            let record = &self.bytes[self.position..self.length];
            // linux_dirent64: inode8、offset8、reclen2、type1、NUL 结尾名称。
            if record.len() < 20 {
                return Err(Failure::Unavailable);
            }
            let length = usize::from(u16::from_ne_bytes([record[16], record[17]]));
            if length < 20 || length > record.len() {
                return Err(Failure::Unavailable);
            }
            let raw = &record[19..length];
            let end = raw
                .iter()
                .position(|v| *v == 0)
                .ok_or(Failure::Unavailable)?;
            let name = &raw[..end];
            self.position += length;
            if name.is_empty() || !name.iter().all(u8::is_ascii_digit) {
                continue;
            }
            let mut number = 0_u32;
            for b in name {
                number = number
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(u32::from(b - b'0')))
                    .ok_or(Failure::Unsupported)?;
            }
            return Ok(Some(number));
        }
    }
}
