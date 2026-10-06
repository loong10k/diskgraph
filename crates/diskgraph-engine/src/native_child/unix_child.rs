#[cfg(any(test, target_os = "macos"))]
use super::ControlWriteStatus;
use super::child_read_buffer::ChildReadBuffer;
use super::unix_child_group::UnixChildGroup;
use super::unix_child_setup::UnixChildSetup;
use super::unix_control_channel::UnixControlChannel;
use super::unix_leader::UnixLeader;
#[cfg(any(test, target_os = "macos"))]
use super::unix_normal_exit::UnixNormalExit;
use super::{ChildError, ChildSpawnError};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, OwnedFd};
use std::process::Child;

#[cfg(target_os = "macos")]
#[path = "unix_child_cleanup_poll.rs"]
mod unix_child_cleanup_poll;
#[path = "unix_child_spawn.rs"]
mod unix_child_spawn;

/// 独占子进程、未回收 leader 与非阻塞双管道，避免旧 PGID 复用误杀。
/// 来源：原生 Rust diskgraph-engine 的 Unix waitid/WNOWAIT 执行边界。
pub(crate) struct UnixChild {
    child: UnixLeader,
    stdout: Option<File>,
    stderr: Option<File>,
    control: Option<UnixControlChannel>,
    #[cfg(any(test, target_os = "macos"))]
    control_closed: bool,
    buffer: ChildReadBuffer,
    stdout_eof: bool,
    stderr_eof: bool,
    exit_code: Option<i32>,
    owns_group: bool,
    cleaned: bool,
    #[cfg(any(test, target_os = "macos"))]
    normal_exit: UnixNormalExit,
    #[cfg(target_os = "macos")]
    _installation_lease:
        Option<std::sync::Arc<crate::macos_installation_lease::MacosInstallationLease>>,
    #[cfg(test)]
    cleanup_fault: bool,
}

impl UnixChild {
    /// 出生前构造全部原资源 owner。参数：为预备缓冲、管道和已认证租约；返回：未生进程。
    #[cfg(target_os = "macos")]
    pub(super) fn prepare_native(
        buffer: ChildReadBuffer,
        control: UnixControlChannel,
        stdout: File,
        stderr: File,
        lease: std::sync::Arc<crate::macos_installation_lease::MacosInstallationLease>,
    ) -> Self {
        Self {
            child: UnixLeader::prepare_native(),
            stdout: Some(stdout),
            stderr: Some(stderr),
            control: Some(control),
            #[cfg(any(test, target_os = "macos"))]
            control_closed: false,
            buffer,
            stdout_eof: false,
            stderr_eof: false,
            exit_code: None,
            owns_group: false,
            cleaned: false,
            normal_exit: UnixNormalExit::Unavailable,
            _installation_lease: Some(lease),
            #[cfg(test)]
            cleanup_fault: false,
        }
    }

    /// 原生 C 直接写入出生前存在的原 owner；参数：是固定路径和三个原子端，返回：原 errno。
    #[cfg(target_os = "macos")]
    pub(super) fn birth_native(&mut self, path: &std::ffi::CStr, channels: [i32; 3]) -> i32 {
        let UnixLeader::Native { pid, .. } = &mut self.child else {
            return libc::EINVAL;
        };
        super::macos_native_spawn::MacosNativeSpawn::birth(
            path,
            channels,
            pid,
            &mut self.owns_group,
        )
    }

    /// 原 owner 存在后核验实际 SID/PGID 与管道；参数：无，返回：真实初始化错误。
    #[cfg(target_os = "macos")]
    pub(super) fn initialize_native(&mut self) -> Result<u32, ChildError> {
        UnixChildSetup::initialize(
            [
                self.stdout.as_ref().map(AsRawFd::as_raw_fd),
                self.stderr.as_ref().map(AsRawFd::as_raw_fd),
            ],
            true,
            self.child.id(),
        )?;
        self.normal_exit = UnixNormalExit::Qualified;
        Ok(self.child.id())
    }

    /// 接管实际 Child 的管道并执行原 post-spawn 检查；不再创建其它 child owner。
    /// 参数：child/control 为独占资源，buffer 为出生前已准备的固定读缓冲，private_session 为旧入口核验，
    /// cleanup_fault 仅保留旧测试注入，checkpoint 借原检查；返回：初始化完成的唯一 owner。
    pub(super) fn from_spawn<E>(
        mut child: Child,
        buffer: ChildReadBuffer,
        control: Option<UnixControlChannel>,
        private_session: bool,
        cleanup_fault: bool,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        #[cfg(not(test))]
        let _ = cleanup_fault;
        // 管道只转移原描述符，不重新打开、不分配；任何 post-spawn 检查前完整 owner 已就位。
        let stdout = child
            .stdout
            .take()
            .map(|pipe| File::from(OwnedFd::from(pipe)));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| File::from(OwnedFd::from(pipe)));
        let mut owner = Self {
            child: UnixLeader::from_standard(child),
            stdout,
            stderr,
            control,
            #[cfg(any(test, target_os = "macos"))]
            control_closed: false,
            buffer,
            stdout_eof: false,
            stderr_eof: false,
            exit_code: None,
            owns_group: true,
            cleaned: false,
            #[cfg(any(test, target_os = "macos"))]
            normal_exit: UnixNormalExit::Unavailable,
            #[cfg(target_os = "macos")]
            _installation_lease: None,
            #[cfg(test)]
            cleanup_fault,
        };
        let initialized: Result<(), ChildSpawnError<E>> = (|| {
            UnixChildSetup::initialize(
                [
                    owner.stdout.as_ref().map(AsRawFd::as_raw_fd),
                    owner.stderr.as_ref().map(AsRawFd::as_raw_fd),
                ],
                private_session,
                owner.child.id(),
            )?;
            #[cfg(any(test, target_os = "macos"))]
            if private_session {
                owner.normal_exit = UnixNormalExit::Qualified;
            }
            checkpoint().map_err(ChildSpawnError::checkpoint)?;
            Ok(())
        })();
        if let Err(error) = initialized {
            return Err(error.with_cleanup(owner.cleanup()));
        }
        Ok(owner)
    }

    /// 在完整退出证据成立后正常 wait，不向组或 leader 发送任何终止信号。
    /// 参数：checkpoint 借调用方原期限/权限检查；返回：true 为正常回收事实，
    /// false 仅表示仍活动或尚未读完管道，未知视图/失去身份明确返回错误。
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn poll_normal_exit<E>(
        &mut self,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<bool, ChildSpawnError<E>> {
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        #[cfg(any(test, target_os = "macos"))]
        if self.normal_exit.is_completed() {
            checkpoint().map_err(ChildSpawnError::checkpoint)?;
            return Ok(true);
        }
        if !self.normal_exit.is_qualified() || self.cleaned || !self.owns_group {
            return Err(
                ChildError::Unsupported("child has no qualified normal exit permission").into(),
            );
        }
        self.normal_exit.validate_platform()?;
        let exited = self.poll()?;
        let ready = exited
            && self.control_closed
            && self.stdout_eof
            && self.stderr_eof
            && self.normal_exit.group_exited(self.child.id())?;
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        if !ready {
            return Ok(false);
        }
        #[cfg(test)]
        super::unix_normal_exit_tests::reap_before_normal_wait(self.child.id());
        if let Err(error) = self.normal_exit.reap(&mut self.child) {
            if matches!(&error, ChildError::NativeIo { source, .. }
                if source.raw_os_error() == Some(libc::ECHILD))
            {
                // 最后wait已明确失去原leader：返回原错误前撤销旧数值组资格。
                // 不等待下一次poll再次访问可能复用的PID，也不把未知owner签发为完成。
                self.owns_group = false;
                self.normal_exit = UnixNormalExit::Unavailable;
            }
            return Err(error.into());
        }
        self.owns_group = false;
        self.cleaned = true;
        self.normal_exit = UnixNormalExit::Completed;
        self.stdout.take();
        self.stderr.take();
        self.control.take();
        Ok(true)
    }

    /// 开始一个有界控制块；Null 不提供写能力，也不创建其它执行 owner。
    /// 参数：bytes 为非空且最多 4096 字节的原始块；返回：实际 Written/Pending 或原生错误。
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn start_control_write(
        &mut self,
        bytes: &[u8],
    ) -> Result<ControlWriteStatus, ChildError> {
        self.control
            .as_mut()
            .ok_or(ChildError::Unsupported("child has no control input"))?
            .start_write(bytes)
    }

    /// 对同一个控制块做一次非阻塞轮询，不重新复制或重发新块。
    /// 参数：无；返回：Written/Pending/Closed，或 Null、空闲状态、实际 I/O 错误。
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn poll_control_write(&mut self) -> Result<ControlWriteStatus, ChildError> {
        self.control
            .as_mut()
            .ok_or(ChildError::Unsupported("child has no control input"))?
            .poll_write()
    }

    /// 停止控制输入并关闭本 owner 的写端，不承诺完整 raw 帧已经发送。
    /// 参数：无；返回：Closed 或 Null 的 Unsupported；后续 EOF 仍由子进程实际读取得证。
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn request_control_close(&mut self) -> Result<ControlWriteStatus, ChildError> {
        let status = self
            .control
            .as_mut()
            .ok_or(ChildError::Unsupported("child has no control input"))?
            .close();
        self.control_closed = matches!(status, ControlWriteStatus::Closed);
        Ok(status)
    }

    /// 观察退出但不回收 leader；其身份保留到进程组清理完成。
    /// 参数：无。
    /// 返回：leader 是否退出，或观察失败；ECHILD 时不再盲杀旧 PGID。
    pub(crate) fn poll(&mut self) -> Result<bool, ChildError> {
        #[cfg(any(test, target_os = "macos"))]
        if self.normal_exit.is_completed() {
            return Ok(true);
        }
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
        read_pipe(pipe, &mut self.stdout_eof, self.buffer.bytes_mut())
            .map(|count| count.map(|count| &self.buffer.bytes()[..count]))
    }

    /// 读取 stderr 的一个固定小块，与 stdout 公平轮转。
    /// 参数：无。
    /// 返回：本轮字节，或暂无数据/EOF，或真实读取错误。
    pub(crate) fn read_stderr(&mut self) -> Result<Option<&[u8]>, ChildError> {
        let pipe = self
            .stderr
            .as_mut()
            .ok_or(ChildError::Unsupported("missing stderr"))?;
        read_pipe(pipe, &mut self.stderr_eof, self.buffer.bytes_mut())
            .map(|count| count.map(|count| &self.buffer.bytes()[..count]))
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
            // 失去等待权不等于完成回收；每次重试都保留失败，防止 registry
            // 将未知所有权误判为 Complete 并释放仍需人工处置的容量。
            return Err(ChildError::Unsupported(
                "child ownership lost; refusing numeric process-group cleanup",
            ));
        }
        // 再查身份：宿主不得全局 auto-reap 或并发 wait 本 owner 的子进程。
        let finished = self.poll()?;
        let pid = i32::try_from(self.child.id())
            .map_err(|_| ChildError::Unsupported("unrepresentable child pid"))?;
        let group = UnixChildGroup::terminate(pid);
        if !finished && unsafe { libc::kill(pid, libc::SIGKILL) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                // leader 仍由本 owner 保留；即使离开原组也只终止其 PID，
                // 不追随新 PGID，避免向宿主或其他样本的进程组发信号。
                return Err(ChildError::io("terminate owned child", error).with_cleanup(group));
            }
        }
        // 组终止失败时不能消费 leader：它是后续重试使用原 PGID 的身份锚点。
        // leader 已终止也继续保留 zombie 等待权，直到整组清理得到确认。
        group?;
        // leader 尚未回收，普通后代仍属于本组；主动 setsid 逃离者不受此约束。
        #[cfg(test)]
        super::unix_normal_exit_tests::reap_before_cleanup_wait(pid);
        match self.child.wait() {
            Ok(_) => {}
            Err(error) => {
                // 初始观察之后仍可能被外部 wait 消费；失败不能签发完成事实。
                // ECHILD 已明确失去旧 PID/PGID 权限，其余错误保留原 owner 待重试。
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.owns_group = false;
                }
                return Err(ChildError::io("reap owned child", error));
            }
        }
        self.cleaned = true;
        self.owns_group = false;
        self.stdout.take();
        self.stderr.take();
        self.control.take();
        Ok(())
    }
}

impl Drop for UnixChild {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
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
