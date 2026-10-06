//! Windows唯一出生实现；新入口使用调用方catch外槽，兼容入口保留旧清理行为。
#[cfg(test)]
use super::super::windows_control_test_witness::WindowsControlTestWitness;
use super::super::{
    attribute_list::AttributeList, overlapped_control_pipe::OverlappedControlPipe,
    overlapped_pipe::OverlappedPipe, owned_handle::OwnedHandle, pipe_security::PipeSecurity,
    windows_birth_phase::WindowsBirthPhase, windows_command_line::WindowsCommandLine,
};
use super::{WindowsChild, last};
use crate::native_child::{ChildError, ChildInputMode, ChildSpawnError};
use std::process::Command;
use std::ptr::null;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW,
};

impl WindowsChild {
    /// 已弃用的受信内部兼容入口；参数：为原命令与检查点，返回：旧child/错误签名。
    /// 该签名不能移交失败owner；永久清理失败及panic没有外部Recovery保证，应迁移spawn_into。
    pub(crate) fn spawn<E>(
        command: &mut Command,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        Self::spawn_with_input(command, ChildInputMode::Null, checkpoint)
    }

    /// 已弃用的受信内部兼容入口，不得用于新扫描或作为远程授权路径。
    /// 既有WindowsProbe尚待迁移；此薄包装不修复其局部owner在失败返回：时的生命周期限制。
    /// 参数：原命令、stdin模式和检查点；返回：旧签名，不宣称清理失败owner可跨返回：保留。
    pub(crate) fn spawn_with_input<E>(
        command: &mut Command,
        mode: ChildInputMode,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<Self, ChildSpawnError<E>> {
        let mut owner = None;
        match Self::spawn_into(command, mode, &mut owner, checkpoint) {
            Ok(()) => Ok(owner.take().expect("successful birth retains its owner")),
            Err(error) => {
                let cleanup = owner.as_mut().map_or(Ok(()), Self::cleanup);
                Err(error.with_cleanup(cleanup))
            }
        }
    }

    /// 在调用者catch外预留槽内出生，不在失败时消费、清理或丢弃原owner。
    /// 参数：owner必须为空且活得比catch更久，checkpoint借原期限/取消；返回：原错误，出生后的owner由调用者处置。
    pub(crate) fn spawn_into<E>(
        command: &mut Command,
        mode: ChildInputMode,
        owner: &mut Option<Self>,
        checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<(), ChildSpawnError<E>> {
        Self::spawn_into_with_admission(command, mode, owner, || Ok(()), checkpoint)
    }

    /// 参数：admission 借原请求期限/取消，checkpoint 保留四阶段生命周期，owner 是 catch 外槽。
    /// 返回：原错误；所有连接仅 FALSE 轮询并复查 admission，不建立新预算或隐藏等待线程。
    /// 不承诺单次原生调用硬期限；旧 spawn_into 的本地兼容 admission 不设期限。
    pub(crate) fn spawn_into_with_admission<E>(
        command: &mut Command,
        mode: ChildInputMode,
        owner: &mut Option<Self>,
        mut admission: impl FnMut() -> Result<(), E>,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<(), ChildSpawnError<E>> {
        if owner.is_some() {
            return Err(ChildError::Unsupported("birth owner slot already occupied").into());
        }
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        let mut input = WindowsCommandLine::from_command(command)?;
        let security = PipeSecurity::for_current_user()?;
        let nonce = uuid::Uuid::new_v4();
        let stdout_name = super::pipe_name(&format!("diskgraph-probe-{nonce}-out"));
        let stderr_name = super::pipe_name(&format!("diskgraph-probe-{nonce}-err"));
        // Job 和确认未出生状态先进入调用者外槽，再提交任何管道 Connect。
        admission().map_err(ChildSpawnError::checkpoint)?;
        Self::prepare_job_into(owner)?;
        let prepared = owner.as_mut().expect("original prepared Job owner");
        let stdout_writer = prepare_read_pipe(
            &stdout_name,
            &security,
            &mut prepared.stdout,
            &mut admission,
        )?;
        let stderr_writer = prepare_read_pipe(
            &stderr_name,
            &security,
            &mut prepared.stderr,
            &mut admission,
        )?;
        admission().map_err(ChildSpawnError::checkpoint)?;
        let stdin = OverlappedControlPipe::prepare_input_into(
            mode,
            &nonce,
            &security,
            &mut prepared.control,
            &mut admission,
        )?;
        let mut attributes = AttributeList::new()?;
        let handles: [HANDLE; 3] = [
            stdin.as_raw(),
            stdout_writer.as_raw(),
            stderr_writer.as_raw(),
        ];
        attributes.set_jobs([prepared.job.as_ref().expect("original Job").as_raw()])?;
        attributes.set_handles(handles)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = attributes.as_raw();
        let mut info = PROCESS_INFORMATION::default();
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        let application = input.application();
        let arguments = input.arguments();
        let environment = input.environment();
        let directory = input.directory();
        admission().map_err(ChildSpawnError::checkpoint)?;
        // 创建结果未知期间禁止把缺失 process 解释为未出生；无用户回调窗口。
        owner.as_mut().expect("original prepared owner").birth_phase = WindowsBirthPhase::Creating;
        if unsafe {
            CreateProcessW(
                application,
                arguments,
                null(),
                null(),
                1,
                CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT,
                environment,
                directory,
                &startup.StartupInfo,
                &mut info,
            )
        } == 0
        {
            let error = last("CreateProcessW(JOB_LIST,HANDLE_LIST)");
            // 原调用明确失败，但 Job/pending I/O 责任仍留外槽，绝不直接标记 cleaned。
            owner
                .as_mut()
                .expect("original failed birth owner")
                .birth_phase = WindowsBirthPhase::CreateFailed;
            return Err(error.into());
        }
        // 成功的Win32调用保证有效process/thread句柄；接管无分配且无用户回调。
        // 后续所有后检、hook及Resume均借用已经位于catch外的原owner。
        let child = owner
            .as_mut()
            .expect("prepared owner remains in external slot");
        child.process = Some(OwnedHandle::from_raw(
            info.hProcess,
            "CreateProcessW(process handle)",
        )?);
        child.birth_phase = WindowsBirthPhase::ProcessOwned;
        let thread = OwnedHandle::from_raw(info.hThread, "CreateProcessW(thread handle)")?;
        // Child 已有且 suspended；父本地写端必须立即关闭，避免 EOF 被自身阻止。
        drop(stdin);
        drop(stdout_writer);
        drop(stderr_writer);
        #[cfg(test)]
        super::super::windows_birth_test_hook::WindowsBirthTestHook::observe(child);
        #[cfg(test)]
        if let (Some(job), Some(process)) = (&child.job, &child.process) {
            WindowsControlTestWitness::record(job.as_raw(), process.as_raw());
        }
        #[cfg(test)]
        if command.get_envs().any(|(key, value)| {
            key.to_string_lossy()
                .eq_ignore_ascii_case("DG_WINDOWS_NATIVE_FAULT")
                && value.is_some_and(|value| value.to_string_lossy() == "post_create")
        }) {
            // 清理前释放本 owner 已完成的线程句柄，保持句柄生命周期明确。
            drop(thread);
            let error = ChildError::Unsupported("injected post-create failure");
            return Err(error.into());
        }
        if let Err(error) = checkpoint() {
            drop(thread);
            return Err(ChildSpawnError::checkpoint(error));
        }
        // 原请求可在挂起出生后撤销；生命周期检查不替代恢复执行前的实时准入。
        // 失败只释放线程句柄，原process/Job仍留catch外owner等待实际清理。
        if let Err(error) = admission() {
            drop(thread);
            return Err(ChildSpawnError::checkpoint(error));
        }
        let previous = unsafe { ResumeThread(thread.as_raw()) };
        let resume_failure = if previous == u32::MAX {
            Some(last("ResumeThread"))
        } else if previous != 1 {
            Some(ChildError::Unsupported(
                "probe thread did not have exactly one suspend count",
            ))
        } else {
            None
        };
        drop(thread);
        if let Some(error) = resume_failure {
            return Err(error.into());
        }
        if let Err(error) = checkpoint() {
            return Err(ChildSpawnError::checkpoint(error));
        }
        Ok(())
    }
}

// 新产品入口借原 admission；兼容入口传无期限本地检查，不挪用生命周期回调。
fn prepare_read_pipe<E>(
    name: &[u16],
    security: &PipeSecurity,
    owner: &mut Option<OverlappedPipe>,
    admission: &mut impl FnMut() -> Result<(), E>,
) -> Result<OwnedHandle, ChildSpawnError<E>> {
    admission().map_err(ChildSpawnError::checkpoint)?;
    OverlappedPipe::prepare_into(name, security, owner)?;
    admission().map_err(ChildSpawnError::checkpoint)?;
    let writer = OverlappedPipe::open_writer(name, security)?;
    admission().map_err(ChildSpawnError::checkpoint)?;
    let pipe = owner.as_mut().expect("original prepared read pipe");
    pipe.start_connect()?;
    complete_prepared_connection(pipe, admission)?;
    Ok(writer)
}

/// 参数：原连接 owner 与借用原预算的 admission；返回：连接完成或原取消/期限错误。
/// 仅 FALSE 查询；原错/panic 后不消费 owner，不重建窗口或分配新缓冲。
pub(super) fn complete_prepared_connection<E>(
    pipe: &mut OverlappedPipe,
    admission: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), ChildSpawnError<E>> {
    loop {
        admission().map_err(ChildSpawnError::checkpoint)?;
        if pipe.connect_ready()? {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    admission().map_err(ChildSpawnError::checkpoint)?;
    Ok(())
}
