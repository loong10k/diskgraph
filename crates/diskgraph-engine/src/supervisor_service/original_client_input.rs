use std::io;

/// 原stdin非阻塞标志的唯一临时责任；来源：PF-06，无Java对等对象。退出还原，不关闭原FD。
pub(super) struct OriginalClientInput {
    flags: libc::c_int,
}
impl OriginalClientInput {
    /// 参数：无；返回：已设置非阻塞的原stdin责任，失败不授予输入读取能力。
    pub(super) fn new() -> io::Result<Self> {
        let flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let owner = Self { flags };
        if unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(owner)
    }
    /// 参数：有限目标缓冲；返回：原字节数、真实EOF或原OS错误，不读取旁路输入。
    pub(super) fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        let count =
            unsafe { libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), bytes.len()) };
        if count < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(count as usize)
        }
    }
}
impl Drop for OriginalClientInput {
    fn drop(&mut self) {
        let _ = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, self.flags) };
    }
}
