use super::ChildError;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

/// 原 child 转交的 pidfd 退出资格；不以可复用的 PID/PGID 查询、发信号或消费其它进程。
/// 来源：Linux P_PIDFD waitid/WNOWAIT、pidfd_poll whole-thread-group exit 与 pidfd_send_signal。
pub(super) struct LinuxScannerExit {
    fd: OwnedFd,
    pid: u32,
}

impl LinuxScannerExit {
    /// 核验实际 direct-child 关系。参数：fd/pid 为原 child 的有界 SCM_RIGHTS 消息；返回：退出资格。
    pub(super) fn new(fd: OwnedFd, pid: u32) -> Result<Self, ChildError> {
        let exit = Self { fd, pid };
        exit.observe(false)?;
        Ok(exit)
    }

    /// 返回资格中的原 child ID，仅与 join 返回的同一个 Child 比较。
    /// 参数：无；返回：不用于重新打开身份或发信号的 PID 标量。
    pub(super) fn pid(&self) -> u32 {
        self.pid
    }

    /// 只观察原 leader，不回收。参数：consume 为是否真实 wait；返回：原退出码/信号退出或仍活动。
    pub(super) fn observe(&self, consume: bool) -> Result<Option<Option<i32>>, ChildError> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let flags = libc::WEXITED
            | if consume {
                0
            } else {
                libc::WNOHANG | libc::WNOWAIT
            };
        if unsafe { libc::waitid(libc::P_PIDFD, self.fd.as_raw_fd() as u32, &mut info, flags) } < 0
        {
            return Err(ChildError::io(
                "observe original scanner pidfd",
                io::Error::last_os_error(),
            ));
        }
        let pid = unsafe { info.si_pid() };
        if pid == 0 {
            return Ok(None);
        }
        if pid as u32 != self.pid {
            return Err(ChildError::Unsupported(
                "scanner pidfd child identity changed",
            ));
        }
        Ok(Some(
            (info.si_code == libc::CLD_EXITED).then(|| unsafe { info.si_status() }),
        ))
    }

    /// 零等待查询整个线程组是否退出。参数：无；返回：真实 pidfd POLLIN，未知/失去关系为错误。
    pub(super) fn ready(&self) -> Result<bool, ChildError> {
        // 先核 retained ownership，外部 reap 即使 pidfd 仍可读也不得继续领取许可。
        self.observe(false)?;
        let mut view = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut view, 1, 0) } < 0 {
            return Err(ChildError::io(
                "poll scanner whole-thread-group exit",
                io::Error::last_os_error(),
            ));
        }
        if view.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(ChildError::Unsupported(
                "scanner pidfd readiness is unknown",
            ));
        }
        Ok(view.revents & libc::POLLIN != 0)
    }

    /// 异常终止并真实 wait 原进程。参数：无；返回：原退出事实或原 OS 错误，绝不信号数字组。
    pub(super) fn terminate_and_reap(&self) -> Result<Option<i32>, ChildError> {
        self.terminate()?;
        self.observe(true)?.ok_or(ChildError::Unsupported(
            "scanner wait produced no exit record",
        ))
    }

    /// 只停止已核验的原进程，不消费退出记录。参数：无；返回：真实 pidfd 信号结果。
    pub(super) fn terminate(&self) -> Result<(), ChildError> {
        if self.observe(false)?.is_some() {
            return Ok(());
        }
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(ChildError::io("terminate original scanner pidfd", error));
            }
        }
        Ok(())
    }
}
