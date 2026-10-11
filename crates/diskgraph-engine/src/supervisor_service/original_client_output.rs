use std::io;
use std::time::Instant;

/// 客户端原stdout/stderr的临时非阻塞写入责任。来源：PF-06有限前台输出；无Java对等对象。
/// 不关闭原描述符，退出时还原原flags；OS单次不可中断I/O不作为严格硬墙钟保证。
pub(super) struct OriginalClientOutput {
    flags: [(libc::c_int, libc::c_int); 2],
}
impl OriginalClientOutput {
    /// 参数：无；返回：保留原flags并设置原两端非阻塞的责任，失败也还原已改变端。
    pub(super) fn new() -> io::Result<Self> {
        let mut flags = [(libc::STDOUT_FILENO, 0), (libc::STDERR_FILENO, 0)];
        for (fd, flag) in &mut flags {
            *flag = unsafe { libc::fcntl(*fd, libc::F_GETFL) };
            if *flag < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        let owner = Self { flags };
        for (fd, flag) in owner.flags {
            if unsafe { libc::fcntl(fd, libc::F_SETFL, flag | libc::O_NONBLOCK) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(owner)
    }
    /// 参数：kind为stdout/stderr，bytes为原有限帧，deadline为原期限；返回：实际完整写入或拒绝。
    pub(super) fn write(&self, kind: u8, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        let fd = match kind {
            1 => libc::STDOUT_FILENO,
            2 => libc::STDERR_FILENO,
            _ => return Err(io::Error::other("invalid original output channel")),
        };
        let mut sent = 0;
        while sent < bytes.len() {
            super::root_installation_path::check(deadline)?;
            let count =
                unsafe { libc::write(fd, bytes[sent..].as_ptr().cast(), bytes.len() - sent) };
            if count > 0 {
                sent += count as usize;
                continue;
            }
            let error = io::Error::last_os_error();
            match error.kind() {
                io::ErrorKind::Interrupted => {}
                io::ErrorKind::WouldBlock => {
                    super::linux_broker_client::poll_until(fd, libc::POLLOUT, deadline)?
                }
                _ => return Err(error),
            }
        }
        Ok(())
    }
}
impl Drop for OriginalClientOutput {
    fn drop(&mut self) {
        for (fd, flag) in self.flags {
            let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flag) };
        }
    }
}
