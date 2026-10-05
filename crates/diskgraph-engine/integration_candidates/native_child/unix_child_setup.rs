use super::ChildError;
use std::io;
use std::os::fd::RawFd;

/// 初始化自有双管道并核验旧 worker 的实际私有 session，不另持 child 或请求预算。
/// 来源：原生 Rust UnixChild 原 post-spawn fcntl 与 SID/PGID 初始化逻辑。
pub(super) struct UnixChildSetup;

impl UnixChildSetup {
    /// 原位置初始化管道和旧 session。参数：pipes 为唯一 owner 借出的 fd，
    /// private_session 为旧 worker 核验要求，pid 为未回收 leader；返回：原 OS/能力错误。
    pub(super) fn initialize(
        pipes: [Option<RawFd>; 2],
        private_session: bool,
        pid: u32,
    ) -> Result<(), ChildError> {
        for fd in pipes {
            let fd = fd.ok_or(ChildError::Unsupported("missing child pipe"))?;
            // 安全性：fd 由唯一 UnixChild 保留，只更改本次管道描述符的非阻塞标志。
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(ChildError::io(
                    "nonblocking pipe",
                    io::Error::last_os_error(),
                ));
            }
        }
        if private_session {
            let pid = i32::try_from(pid)
                .map_err(|_| ChildError::Unsupported("unrepresentable child pid"))?;
            // 安全性：leader 尚未 wait；保留旧入口实际 SID/PGID 检查及原错误。
            if unsafe { libc::getsid(pid) } != pid || unsafe { libc::getpgid(pid) } != pid {
                return Err(ChildError::Unsupported(
                    "worker private session was not retained",
                ));
            }
        }
        Ok(())
    }

    /// 核对宿主保留 child 的能力，不修改全进程信号配置。
    /// 参数：无；返回：原 SIGCHLD 查询错误或明确 auto-reap Unsupported。
    pub(super) fn reject_auto_reap() -> Result<(), ChildError> {
        // 只读取宿主配置，不改变全进程 SIGCHLD。忽略信号会失去保留身份的能力。
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) } < 0 {
            return Err(ChildError::io(
                "inspect SIGCHLD",
                io::Error::last_os_error(),
            ));
        }
        if action.sa_sigaction == libc::SIG_IGN || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
            return Err(ChildError::Unsupported("host auto-reaps child processes"));
        }
        Ok(())
    }
}
