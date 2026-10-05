use super::{ChildError, ChildSpawnError};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};

/// 独占子进程、未回收 leader 与非阻塞双管道，避免旧 PGID 复用误杀。
/// 来源：原生 Rust diskgraph-engine 的 Unix waitid/WNOWAIT 执行边界。
pub(crate) struct UnixChild {
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    buffer: [u8; 4096],
    stdout_eof: bool,
    stderr_eof: bool,
    exit_code: Option<i32>,
    owns_group: bool,
    cleaned: bool,
    #[cfg(test)]
    cleanup_fault: bool,
}

impl UnixChild {
    /// 在执行前建立独立进程组，立即接管资源并将两管道设为非阻塞。
    /// 参数：command 为结构化受信命令，checkpoint 借用调用方原检查，不创建预算。
    /// 返回：本次进程组 owner，或能力/启动/管道错误。
    pub(crate) fn spawn<E>(
        command: &mut Command,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        reject_auto_reap()?;
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command
            .spawn()
            .map_err(|error| ChildError::io("spawn", error))?;
        let mut owner = Self {
            child,
            stdout: None,
            stderr: None,
            buffer: [0; 4096],
            stdout_eof: false,
            stderr_eof: false,
            exit_code: None,
            owns_group: true,
            cleaned: false,
            #[cfg(test)]
            cleanup_fault: command
                .get_envs()
                .any(|(key, value)| key == "DG_PROBE_CLEANUP_FAULT" && value.is_some()),
        };
        owner.stdout = owner.child.stdout.take();
        owner.stderr = owner.child.stderr.take();
        let initialized: Result<(), ChildSpawnError<E>> = (|| {
            for fd in [
                owner.stdout.as_ref().map(AsRawFd::as_raw_fd),
                owner.stderr.as_ref().map(AsRawFd::as_raw_fd),
            ] {
                let fd = fd.ok_or(ChildError::Unsupported("missing child pipe"))?;
                // 安全性：fd 是 owner 持有的管道，只更改本次描述符的非阻塞标志。
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags < 0
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                {
                    return Err(
                        ChildError::io("nonblocking pipe", io::Error::last_os_error()).into(),
                    );
                }
            }
            checkpoint().map_err(ChildSpawnError::checkpoint)?;
            Ok(())
        })();
        if let Err(error) = initialized {
            return Err(error.with_cleanup(owner.cleanup()));
        }
        Ok(owner)
    }

    /// 观察退出但不回收 leader；其身份保留到进程组清理完成。
    /// 参数：无。
    /// 返回：leader 是否退出，或观察失败；ECHILD 时不再盲杀旧 PGID。
    pub(crate) fn poll(&mut self) -> Result<bool, ChildError> {
        // 安全性：siginfo 输出可零初始化；只观察本次 child，不使用 wait(-1)。
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                self.owns_group = false;
            }
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(ChildError::io("observe retained child ownership", error));
        }
        // 安全性：waitid 成功后，si_pid/si_status 是 CLD 记录字段。
        if unsafe { info.si_pid() } == 0 {
            return Ok(false);
        }
        self.exit_code = (info.si_code == libc::CLD_EXITED).then(|| unsafe { info.si_status() });
        Ok(true)
    }

    /// 读取 stdout 的一个固定小块，永不等待更多数据。
    /// 参数：无。
    /// 返回：本轮字节，或暂无数据/EOF，或真实读取错误。
    pub(crate) fn read_stdout(&mut self) -> Result<Option<&[u8]>, ChildError> {
        let pipe = self
            .stdout
            .as_mut()
            .ok_or(ChildError::Unsupported("missing stdout"))?;
        read_pipe(pipe, &mut self.stdout_eof, &mut self.buffer)
            .map(|count| count.map(|count| &self.buffer[..count]))
    }

    /// 读取 stderr 的一个固定小块，与 stdout 公平轮转。
    /// 参数：无。
    /// 返回：本轮字节，或暂无数据/EOF，或真实读取错误。
    pub(crate) fn read_stderr(&mut self) -> Result<Option<&[u8]>, ChildError> {
        let pipe = self
            .stderr
            .as_mut()
            .ok_or(ChildError::Unsupported("missing stderr"))?;
        read_pipe(pipe, &mut self.stderr_eof, &mut self.buffer)
            .map(|count| count.map(|count| &self.buffer[..count]))
    }

    /// 查询 stdout 是否实际读到 EOF，不用空额度模拟结束。
    /// 参数：无。
    /// 返回：该管道实际结束时 true。
    pub(crate) fn stdout_eof(&self) -> bool {
        self.stdout_eof
    }

    /// 查询 stderr 是否实际读到 EOF。
    /// 参数：无。
    /// 返回：该管道实际结束时 true。
    pub(crate) fn stderr_eof(&self) -> bool {
        self.stderr_eof
    }

    /// 取得未回收 leader 的退出码，信号结束保持 None。
    /// 参数：无。
    /// 返回：正常退出码或无正常退出码。
    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// 清理自有组后回收 leader；安全回收可能超过协作采样期限。
    /// 参数：无。
    /// 返回：完成本次资源清理，或明确的所有权/清理错误。
    pub(crate) fn cleanup(&mut self) -> Result<(), ChildError> {
        if self.cleaned {
            return Ok(());
        }
        #[cfg(test)]
        if self.cleanup_fault {
            // 故障注入保留真实 child/group/管道；Drop 再次清理以安全收场。
            self.cleanup_fault = false;
            return Err(ChildError::Io("injected cleanup failure".into()));
        }
        if !self.owns_group {
            self.cleaned = true;
            return Err(ChildError::Unsupported(
                "child ownership lost; refusing numeric process-group cleanup",
            ));
        }
        // 再查身份：宿主不得全局 auto-reap 或并发 wait 本 owner 的子进程。
        let finished = self.poll()?;
        let pid = i32::try_from(self.child.id())
            .map_err(|_| ChildError::Unsupported("unrepresentable child pid"))?;
        let group = terminate_group(pid);
        if !finished && unsafe { libc::kill(pid, libc::SIGKILL) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                // leader 仍由本 owner 保留；即使离开原组也只终止其 PID，
                // 不追随新 PGID，避免向宿主或其他样本的进程组发信号。
                return Err(ChildError::io("terminate owned child", error).with_cleanup(group));
            }
        }
        // leader 尚未回收，普通后代仍属于本组；主动 setsid 逃离者不受此约束。
        let waited = self
            .child
            .wait()
            .map_err(|error| ChildError::io("reap owned child", error));
        self.cleaned = true;
        self.owns_group = false;
        self.stdout.take();
        self.stderr.take();
        match (group, waited) {
            (Err(error), result) => Err(error.with_cleanup(result.map(|_| ()))),
            (Ok(()), result) => result.map(|_| ()),
        }
    }
}

fn terminate_group(pid: i32) -> Result<(), ChildError> {
    if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if error.raw_os_error() == Some(libc::EPERM) {
        return retry_darwin_group(pid);
    }
    Err(ChildError::io("terminate owned process group", error))
}

#[cfg(target_os = "macos")]
fn retry_darwin_group(pid: i32) -> Result<(), ChildError> {
    let retry_until = std::time::Instant::now() + std::time::Duration::from_millis(50);
    loop {
        if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        if error.raw_os_error() == Some(libc::EPERM) {
            // Darwin 退出过渡可能仍为 SRUN/INEXIT。仅最终稳定 SZOMB 证明
            // 可以消除该错误；未知或真实权限错误在有界重试后仍明确失败。
            if super::macos_child_group::zombies_only(pid as u32) {
                return Ok(());
            }
            if std::time::Instant::now() < retry_until {
                std::thread::sleep(std::time::Duration::from_millis(2));
                continue;
            }
        }
        return Err(ChildError::io("terminate owned process group", error));
    }
}

impl Drop for UnixChild {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn reject_auto_reap() -> Result<(), ChildError> {
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

fn read_pipe<T: Read>(
    pipe: &mut T,
    eof: &mut bool,
    buffer: &mut [u8],
) -> Result<Option<usize>, ChildError> {
    if *eof {
        return Ok(None);
    }
    match pipe.read(buffer) {
        Ok(0) => {
            *eof = true;
            Ok(None)
        }
        Ok(count) => Ok(Some(count)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(ChildError::io("read child pipe", error)),
    }
}
