use std::io;

/// 用户executor原入口的exec后隐私收敛；来源：PF-06 OS权限保真，无Java对等对象。
/// 不创建身份或镜像信任；受信出生层必须先确认不可读取的原镜像和安全内核策略。
pub struct LinuxExecutorPrivacy;

impl LinuxExecutorPrivacy {
    /// 参数：无，作用于调用者原进程；返回：原受保护0/2状态已收敛为0，或原内核拒绝。
    /// 必须先于Engine、会话与子进程初始化；状态1拒绝，不对已暴露进程事后补发安全证明。
    pub fn enter() -> io::Result<()> {
        let result = unsafe { dg_linux_executor_seal_privacy() };
        if result < 0 {
            return Err(io::Error::from_raw_os_error((-result) as i32));
        }
        Ok(())
    }
}

unsafe extern "C" {
    fn dg_linux_executor_seal_privacy() -> libc::c_long;
}
