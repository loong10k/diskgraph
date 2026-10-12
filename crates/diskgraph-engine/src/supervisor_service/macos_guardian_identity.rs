use std::io;

/// macOS guardian 的永久专用服务身份切换；原 executor 继续使用客户身份。
/// 来源：原生 Rust PF-06，Darwin setgroups/setgid/setuid；无 Java 对等对象。
pub struct MacosGuardianIdentity;

impl MacosGuardianIdentity {
    /// 将原 guardian 永久切换到安装配置指定的服务用户与唯一服务组。
    /// 参数：uid/gid 为可信安装者指定的非 root 身份；返回：完整复核结果。
    /// 失败可能已经完成部分降权，调用方不得进入业务分发或丢弃原资源责任。
    ///
    /// # Safety
    /// 只能在独立 guardian 的单线程启动入口调用，所有 executor 已由原 owner 保管。
    /// 不得在 Engine、异步运行时、宿主应用或已有业务线程的进程中调用。
    pub unsafe fn enter(uid: u32, gid: u32) -> io::Result<()> {
        if uid == 0 || gid == 0 || uid == u32::MAX || gid == u32::MAX {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return Err(io::Error::from_raw_os_error(libc::EPERM));
        }
        // Darwin 的组列表保留唯一服务主组；先清除继承组，再永久设置三组身份。
        let groups = [gid as libc::gid_t];
        if unsafe { libc::setgroups(groups.len() as libc::c_int, groups.as_ptr()) } != 0
            || unsafe { libc::setgid(gid) } != 0
            || unsafe { libc::setuid(uid) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut observed = [0 as libc::gid_t; 1];
        if unsafe { libc::getuid() } != uid
            || unsafe { libc::geteuid() } != uid
            || unsafe { libc::getgid() } != gid
            || unsafe { libc::getegid() } != gid
            || unsafe { libc::getgroups(1, observed.as_mut_ptr()) } != 1
            || observed != groups
        {
            return Err(io::Error::other("guardian service identity unconfirmed"));
        }
        // 确认原 root saved-ID 无法恢复；成功反而是准入失败，不能继续交付。
        if unsafe { libc::seteuid(0) } == 0
            || io::Error::last_os_error().raw_os_error() != Some(libc::EPERM)
        {
            return Err(io::Error::other(
                "guardian root identity remains recoverable",
            ));
        }
        Ok(())
    }
}
