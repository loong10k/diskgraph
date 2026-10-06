//! Unix child 的结构化出生入口；等待授权与生命周期检查分别路由。
use super::super::child_read_buffer::ChildReadBuffer;
use super::super::unix_child_setup::UnixChildSetup;
use super::super::unix_control_channel::UnixControlChannel;
use super::super::unix_normal_exit::UnixNormalExit;
use super::super::{ChildError, ChildInputMode, ChildSpawnError};
use super::UnixChild;
use std::ffi::OsStr;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

impl UnixChild {
    /// 在执行前建立独立进程组，立即接管资源并将两管道设为非阻塞。
    /// 参数：command 为结构化受信命令，checkpoint 借用调用方原检查，不创建预算。
    /// 返回：本次进程组 owner，或能力/启动/管道错误。
    pub(crate) fn spawn<E>(
        command: &mut Command,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        Self::spawn_with_input(command, ChildInputMode::Null, checkpoint)
    }

    /// 按显式输入模式启动独占 child，保持旧 Null 的两次检查及清理顺序。
    /// 参数：command 为受信命令，input_mode 为 Null/WorkerControl，checkpoint 借原调用方检查。
    /// 返回：唯一 child owner，或原检查错误与原生启动/清理错误。
    /// 此可信兼容入口等待短创建门，不提供等待期限；产品请求使用独立 admission 入口。
    pub(crate) fn spawn_with_input<E>(
        command: &mut Command,
        input_mode: ChildInputMode,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        Self::spawn_command(command, input_mode, false, || Ok(()), checkpoint)
    }

    /// 原授权探针入口；参数：受信命令、原预算检查。返回：原 owner 或未经替换的错误。
    /// 等待和生命周期都借同一原检查，不创建新预算；检查不得按调用次数推断出生阶段。
    pub(crate) fn spawn_checked<E>(
        command: &mut Command,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        let checkpoint = std::cell::RefCell::new(checkpoint);
        Self::spawn_with_input_and_admission(
            command,
            ChildInputMode::Null,
            || (*checkpoint.borrow_mut())(),
            || (*checkpoint.borrow_mut())(),
        )
    }

    /// 参数：admission 用于门等待授权/期限，checkpoint 为出生前后生命周期检查。
    /// 返回：唯一 owner 或原类型错误；等待检查不推进生命周期回调。
    pub(crate) fn spawn_with_input_and_admission<E>(
        command: &mut Command,
        input_mode: ChildInputMode,
        admission: impl FnMut() -> Result<(), E>,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        Self::spawn_command(command, input_mode, false, admission, checkpoint)
    }

    /// 内部新建 Command 并建立私有 session；不给调用方注入 pre_exec 的入口。
    /// 参数：program、args、environment 为受信本地结构化配置；checkpoint 借原检查。
    /// 返回：具有正常退出资格的 WorkerControl owner，或原始检查/原生错误。
    pub(crate) fn spawn_worker<E>(
        program: &Path,
        args: &[&OsStr],
        environment: &[(&OsStr, &OsStr)],
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        let mut command = Command::new(program);
        command.args(args).envs(environment.iter().copied());
        // 安全性：fresh Command 未设置 process_group；子进程 exec 前只调用
        // async-signal-safe setsid/getpid/getpgrp/getsid，不分配或修改父信号配置。
        unsafe {
            command.pre_exec(|| {
                let pid = libc::getpid();
                if libc::setsid() != pid {
                    return Err(io::Error::last_os_error());
                }
                if libc::getpgrp() != pid || libc::getsid(0) != pid {
                    return Err(io::Error::from_raw_os_error(libc::EPERM));
                }
                Ok(())
            });
        }
        let checkpoint = std::cell::RefCell::new(checkpoint);
        Self::spawn_command(
            &mut command,
            ChildInputMode::WorkerControl,
            true,
            || (*checkpoint.borrow_mut())(),
            || (*checkpoint.borrow_mut())(),
        )
    }

    fn spawn_command<E>(
        command: &mut Command,
        input_mode: ChildInputMode,
        private_session: bool,
        admission: impl FnMut() -> Result<(), E>,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        UnixChildSetup::reject_auto_reap()?;
        let buffer = ChildReadBuffer::new()?;
        #[cfg(target_os = "macos")]
        let mut admission = admission;
        #[cfg(target_os = "macos")]
        let gate = super::super::native_birth_gate::NativeBirthGate::acquire(&mut admission)
            .map_err(ChildSpawnError::checkpoint)?;
        #[cfg(not(target_os = "macos"))]
        let _ = admission;
        let (control, stdin) = match input_mode {
            ChildInputMode::Null => (None, Stdio::null()),
            ChildInputMode::WorkerControl => {
                let (control, stdin) = UnixControlChannel::pair()?;
                (Some(control), stdin)
            }
        };
        if !private_session {
            command.process_group(0);
        }
        command
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command
            .spawn()
            .map_err(|error| ChildError::io("spawn", error));
        // Command 保留的 child stdin 副本必须在成功/失败路径都及时释放，
        // 否则 peer close 会被父侧 read endpoint 掩盖，输入关闭也可能无法交付 EOF。
        command.stdin(Stdio::null());
        #[cfg(target_os = "macos")]
        drop(gate);
        let child = child?;
        #[cfg(test)]
        let cleanup_fault = command
            .get_envs()
            .any(|(key, value)| key == "DG_PROBE_CLEANUP_FAULT" && value.is_some());
        #[cfg(not(test))]
        let cleanup_fault = false;
        Self::from_spawn(
            child,
            buffer,
            control,
            private_session,
            UnixNormalExit::Unavailable,
            cleanup_fault,
            checkpoint,
        )
    }
}
