use super::ChildError;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

/// 独占内核出生时返还的 pidfd，等待整个原线程组而非可复用数字 PID。
/// 来源：Linux CLONE_PIDFD、P_PIDFD/WNOWAIT、pidfd poll 与 pidfd_send_signal。
pub(super) struct LinuxAtomicExit {
    fd: OwnedFd,
    pid: i32,
    reaped: bool,
    record_error: Option<&'static str>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
}

impl LinuxAtomicExit {
    /// 参数：fd、pid 为内核原子返回的原材料；返回：无分配、无检查点的唯一退出 owner。
    pub(super) fn adopt(fd: OwnedFd, pid: i32) -> Self {
        Self {
            fd,
            pid,
            reaped: false,
            record_error: None,
            exit_code: None,
            exit_signal: None,
        }
    }

    /// 参数：无；返回：只用于实际句柄操作的原始 pidfd，不重新打开数值身份。
    pub(super) fn fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// 参数：无；返回：仅诊断用出生 PID，不用于信号或身份重新打开。
    #[cfg(test)]
    pub(super) fn pid(&self) -> i32 {
        self.pid
    }

    /// 参数：无；返回：实际 whole-thread-group readiness；未知、外部 reap 或原 OS 错误失败。
    pub(super) fn ready(&mut self) -> Result<bool, ChildError> {
        if let Some(reason) = self.record_error {
            return Err(ChildError::Unsupported(reason));
        }
        if self.reaped {
            return Ok(true);
        }
        self.observe(false)?;
        #[cfg(test)]
        super::linux_atomic_launcher_ready_tests::after_observe(self.exit_code, self.exit_signal);
        let mut view = libc::pollfd {
            fd: self.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut view, 1, 0) } < 0 {
            return Err(ChildError::io(
                "poll original atomic pidfd",
                io::Error::last_os_error(),
            ));
        }
        if view.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(ChildError::Unsupported(
                "original atomic pidfd readiness unknown",
            ));
        }
        if view.revents & libc::POLLIN == 0 {
            return Ok(false);
        }
        // child 可在首次 WNOHANG 与 poll 之间退出；readiness 后重新观察原 pidfd，
        // 仍用 WNOWAIT，不消费原 owner 的 wait，也不把可读性当作已知退出记录。
        self.observe(false)?;
        if self.exit_code.is_none() && self.exit_signal.is_none() {
            return Err(ChildError::Unsupported(
                "readable atomic pidfd has no observed exit record",
            ));
        }
        Ok(true)
    }

    /// 参数：无；返回：真实原线程组退出并实际消费原 child 的结果，不调用 kill。
    pub(super) fn reap_normal(&mut self) -> Result<(), ChildError> {
        if let Some(reason) = self.record_error {
            return Err(ChildError::Unsupported(reason));
        }
        if self.reaped {
            return Ok(());
        }
        if !self.ready()? {
            return Err(ChildError::Unsupported("atomic child is still active"));
        }
        self.observe(true)?;
        Ok(())
    }

    /// 参数：无；返回：异常终止和真实 wait 的结果；ECHILD 不回退数字 PID。
    pub(super) fn cleanup(&mut self) -> Result<(), ChildError> {
        if let Some(reason) = self.record_error {
            return Err(ChildError::Unsupported(reason));
        }
        if self.reaped {
            return Ok(());
        }
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(ChildError::io("terminate original atomic pidfd", error));
            }
        }
        self.observe(true)?;
        Ok(())
    }

    /// 参数：deadline 为宿主绝对期限；返回：原整个线程组停止且原等待实际消费。
    /// 每笔 waitid 使用 WNOHANG；EINTR 返回 Pending，不重试，不回退数值 PID。
    pub(super) fn poll_cleanup(
        &mut self,
        deadline: std::time::Instant,
    ) -> Result<bool, ChildError> {
        if let Some(reason) = self.record_error {
            return Err(ChildError::Unsupported(reason));
        }
        if self.reaped {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(ChildError::io("terminate original atomic pidfd", error));
            }
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        // 先保留原 wait 身份；真实 ECHILD/权限拒绝不能由物理退出观察掩盖。
        if self.observe_once(false)?.is_none() {
            return Ok(false);
        }
        if std::time::Instant::now() >= deadline || !self.stopped()? {
            return Ok(false);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        Ok(self.observe_once(true)?.unwrap_or(false))
    }

    fn observe_once(&mut self, consume: bool) -> Result<Option<bool>, ChildError> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let options = libc::WEXITED | libc::WNOHANG | if consume { 0 } else { libc::WNOWAIT };
        if unsafe { libc::waitid(libc::P_PIDFD, self.fd() as u32, &mut info, options) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                return Ok(None);
            }
            return Err(ChildError::io("poll wait original atomic pidfd", error));
        }
        if unsafe { info.si_pid() } == 0 {
            return Ok(Some(false));
        }
        self.record(&info, consume)?;
        Ok(Some(true))
    }

    /// 参数：无；返回：真实普通退出码，信号退出没有伪造普通码。
    pub(super) fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// 参数：无；返回：真实终止信号，仅在实际退出记录存在时返回。
    #[cfg(test)]
    pub(super) fn exit_signal(&self) -> Option<i32> {
        self.exit_signal
    }

    /// 参数：无；返回：实际消费 wait 完成事实，不以 readiness 假造已回收。
    pub(super) fn reaped(&self) -> bool {
        self.reaped
    }

    /// 参数：无；返回：纯原 pidfd 的物理退场事实，不替代原wait消费或正常许可。
    pub(super) fn stopped(&self) -> Result<bool, ChildError> {
        if let Some(reason) = self.record_error {
            return Err(ChildError::Unsupported(reason));
        }
        if self.reaped {
            return Ok(true);
        }
        let mut view = libc::pollfd {
            fd: self.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut view, 1, 0) } < 0 {
            return Err(ChildError::io(
                "inspect failed atomic child exit",
                io::Error::last_os_error(),
            ));
        }
        if view.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(ChildError::Unsupported(
                "failed atomic child exit is unknown",
            ));
        }
        Ok(view.revents & libc::POLLIN != 0)
    }

    fn observe(&mut self, consume: bool) -> Result<(), ChildError> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let options = libc::WEXITED
            | if consume {
                0
            } else {
                libc::WNOWAIT | libc::WNOHANG
            };
        loop {
            if unsafe { libc::waitid(libc::P_PIDFD, self.fd() as u32, &mut info, options) } == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(ChildError::io("wait original atomic pidfd", error));
            }
        }
        let pid = unsafe { info.si_pid() };
        if pid == 0 {
            if consume {
                return Err(ChildError::Unsupported("atomic wait missing exit record"));
            }
            return Ok(());
        }
        self.record(&info, consume)
    }

    fn record(&mut self, info: &libc::siginfo_t, consume: bool) -> Result<(), ChildError> {
        // 实际消费不可撤回；异常记录须同时保存事实与拒绝状态，后续不能假报 Complete。
        if consume {
            self.reaped = true;
        }
        let reason = if unsafe { info.si_pid() } != self.pid {
            Some("atomic pidfd child identity mismatch")
        } else {
            let status = unsafe { info.si_status() };
            match info.si_code {
                libc::CLD_EXITED => {
                    self.exit_code = Some(status);
                    None
                }
                libc::CLD_KILLED | libc::CLD_DUMPED => {
                    self.exit_signal = Some(status);
                    None
                }
                _ => Some("atomic wait exit status unknown"),
            }
        };
        if let Some(reason) = reason {
            if consume {
                self.record_error = Some(reason);
            }
            return Err(ChildError::Unsupported(reason));
        }
        Ok(())
    }
}
