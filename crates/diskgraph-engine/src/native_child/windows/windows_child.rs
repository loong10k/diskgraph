//! Win10+ 原生 CreateProcessW：进程在运行前即绑定本任务 Job。

use std::io;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
#[cfg(test)]
use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
#[cfg(test)]
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
    QueryInformationJobObject, TerminateJobObject,
};
#[cfg(test)]
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};

use super::super::{ChildError, ChildSpawnError, ControlWriteStatus};
use super::overlapped_control_pipe::OverlappedControlPipe;
use super::overlapped_pipe::OverlappedPipe;
use super::owned_handle::OwnedHandle;
use super::windows_birth_phase::WindowsBirthPhase;
use super::windows_normal_exit::WindowsNormalExit;

/// 本次 Windows 子进程的唯一 Job、leader、两条读管道与可选控制输入。来源：Win32 CreateProcessW、Job Objects 与 Overlapped I/O。
pub(crate) struct WindowsChild {
    job: Option<OwnedHandle>,
    process: Option<OwnedHandle>,
    stdout: Option<OverlappedPipe>,
    stderr: Option<OverlappedPipe>,
    birth_phase: WindowsBirthPhase,
    control: Option<OverlappedControlPipe>,
    normal_exit: WindowsNormalExit,
    exit: Option<i32>,
    cleaned: bool,
}

#[path = "windows_child_poll_cleanup.rs"]
mod windows_child_poll_cleanup;
#[path = "windows_child_prepare.rs"]
mod windows_child_prepare;
#[cfg(test)]
#[path = "windows_child_prepared_tests.rs"]
mod windows_child_prepared_tests;
#[path = "windows_child_spawn.rs"]
mod windows_child_spawn;
#[cfg(test)]
#[path = "windows_image_policy_tests.rs"]
mod windows_image_policy_tests;

impl WindowsChild {
    /// 发起单个固定大小控制块。参数：bytes 非空且最多 4096 字节。返回：Written、Pending 或原错；Null 模式明确不支持。
    pub(crate) fn start_control_write(
        &mut self,
        bytes: &[u8],
    ) -> Result<ControlWriteStatus, ChildError> {
        self.control
            .as_mut()
            .ok_or(ChildError::Unsupported("control input mode is Null"))?
            .start_write(bytes)
    }

    /// 只轮询当前自有控制块。参数：无。返回：真实完成字节数、Pending 或 Closed；不续期、不等待。
    pub(crate) fn poll_control_write(&mut self) -> Result<ControlWriteStatus, ChildError> {
        let result = self
            .control
            .as_mut()
            .ok_or(ChildError::Unsupported("control input mode is Null"))?
            .poll_write();
        self.normal_exit.record_control(&result);
        result
    }

    /// 封闭后续控制写入。参数：无。返回：Closed 或保留内核借用内存的 Pending，不保证帧已 flush。
    pub(crate) fn request_control_close(&mut self) -> Result<ControlWriteStatus, ChildError> {
        let result = self
            .control
            .as_mut()
            .ok_or(ChildError::Unsupported("control input mode is Null"))?
            .request_close();
        self.normal_exit.record_control(&result);
        result
    }

    /// 借用真正 pending 操作供原生测试核验。参数：无。返回：原写句柄、OVERLAPPED 和 buffer 地址，不注入状态。
    #[cfg(test)]
    pub(crate) fn control_io_witness_for_test(
        &self,
    ) -> Result<(HANDLE, *const OVERLAPPED, *const u8), ChildError> {
        self.control
            .as_ref()
            .ok_or(ChildError::Unsupported("control input mode is Null"))?
            .io_witness()
    }

    /// 借用父控制句柄供继承测试。参数：无。返回：原父写端与 event；不转移、不复制所有权。
    #[cfg(test)]
    pub(crate) fn control_handles_for_test(&self) -> Result<(HANDLE, HANDLE), ChildError> {
        self.control
            .as_ref()
            .ok_or(ChildError::Unsupported("control input mode is Null"))?
            .handles()
    }

    /// 观察 leader 是否已退出。参数：无。返回：true 表示已记录退出码，错误表示 Win32 查询失败。
    pub(crate) fn poll(&mut self) -> Result<bool, ChildError> {
        if self.exit.is_some() {
            return Ok(true);
        }
        let process = self
            .process
            .as_ref()
            .ok_or(ChildError::Unsupported("probe leader already cleaned"))?;
        match unsafe { WaitForSingleObject(process.as_raw(), 0) } {
            WAIT_TIMEOUT => Ok(false),
            WAIT_OBJECT_0 => {
                let mut code = 0u32;
                if unsafe { GetExitCodeProcess(process.as_raw(), &mut code) } == 0 {
                    return Err(last("GetExitCodeProcess"));
                }
                self.exit = Some(code as i32);
                Ok(true)
            }
            _ => Err(last("WaitForSingleObject(probe leader)")),
        }
    }

    /// 查询已记录的 leader 退出码。参数：无。返回：未观察退出时为 None，否则为进程退出码。
    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.exit
    }

    /// 以原检查点观察正常整 Job 完成，不终止或清理。参数：checkpoint 为原期限／授权检查。返回：leader、双 EOF、控制 Closed 和 Job 活动零全部成立时 true；原检查点错误保留且不隐含 cleanup，原退出码不改写。
    pub(crate) fn poll_normal_exit<E>(
        &mut self,
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<bool, ChildSpawnError<E>> {
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        if self.control.is_none() {
            return Err(ChildError::Unsupported("normal exit requires WorkerControl input").into());
        }
        let leader_exited = self.poll()?;
        let complete = self.normal_exit.poll(
            self.job.as_ref(),
            leader_exited,
            self.stdout_eof() && self.stderr_eof(),
        )?;
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        Ok(complete)
    }

    /// 非阻塞读取 stdout 的固定片段。参数：无。返回：有数据时为最多 4096 字节，否则为 None 或错误。
    pub(crate) fn read_stdout(&mut self) -> Result<Option<&[u8]>, ChildError> {
        self.stdout
            .as_mut()
            .ok_or(ChildError::Unsupported("stdout absent during preparation"))?
            .read_next()
    }

    /// 非阻塞读取 stderr 的固定片段。参数：无。返回：有数据时为最多 4096 字节，否则为 None 或错误。
    pub(crate) fn read_stderr(&mut self) -> Result<Option<&[u8]>, ChildError> {
        self.stderr
            .as_mut()
            .ok_or(ChildError::Unsupported("stderr absent during preparation"))?
            .read_next()
    }

    /// 查询 stdout 是否已确认 EOF。参数：无。返回：确认 EOF 时为 true。
    pub(crate) fn stdout_eof(&self) -> bool {
        self.stdout.as_ref().is_some_and(OverlappedPipe::eof)
    }

    /// 查询 stderr 是否已确认 EOF。参数：无。返回：确认 EOF 时为 true。
    pub(crate) fn stderr_eof(&self) -> bool {
        self.stderr.as_ref().is_some_and(OverlappedPipe::eof)
    }

    /// 为 Windows 原生验收复制本 Job 句柄，便于清理前后直接核验活动进程数。
    /// 参数：无。返回：测试独有的 Job 句柄所有权或 Win32 错误。
    #[cfg(test)]
    pub(crate) fn duplicate_job_for_test(&self) -> Result<OwnedHandle, ChildError> {
        let job = self
            .job
            .as_ref()
            .ok_or(ChildError::Unsupported("probe Job already cleaned"))?;
        duplicate_handle_for_test(job.as_raw(), "DuplicateHandle(test Job)")
    }

    /// 为 Windows 原生验收复制 leader 句柄，模拟外部宿主保持已退出进程引用。
    /// 参数：无。返回：测试独有的进程句柄所有权或 Win32 错误。
    #[cfg(test)]
    pub(crate) fn duplicate_leader_for_test(&self) -> Result<OwnedHandle, ChildError> {
        let process = self
            .process
            .as_ref()
            .ok_or(ChildError::Unsupported("probe leader already cleaned"))?;
        duplicate_handle_for_test(process.as_raw(), "DuplicateHandle(test leader)")
    }

    /// 清理本次 Job 与所有普通后代并回收 pending I/O。
    /// 参数：无。返回：Job 活动进程归零且管道回收成功，或可包含多项原因的清理错误。
    /// Job 归零只观察一秒；leader 等待与 pending I/O 安全回收仍可能超过协作期限。
    pub(crate) fn cleanup(&mut self) -> Result<(), ChildError> {
        if self.cleaned {
            return Ok(());
        }
        let mut failure = None;
        // 整次清理成功前只借用原责任句柄；失败后的 Recovery 必须能再次真实观察。
        let terminated = if let Some(active_job) = self.job.as_ref() {
            if unsafe { TerminateJobObject(active_job.as_raw(), 1) } == 0 {
                record_failure(&mut failure, last("TerminateJobObject"));
                false
            } else {
                true
            }
        } else {
            record_failure(&mut failure, ChildError::Unsupported("cleanup Job missing"));
            false
        };
        // 终止请求失败不能无界等待仍运行的 leader，也不能关闭 Job 丢失终态观察能力。
        // 即使本轮 leader 等待成功，也将原句柄保留到全部阶段成功。
        if terminated && let Some(process) = self.process.as_ref() {
            #[cfg(test)]
            let waited = super::windows_cleanup_hooks::WindowsCleanupHooks::wait(|| unsafe {
                WaitForSingleObject(process.as_raw(), INFINITE)
            });
            #[cfg(not(test))]
            let waited = unsafe { WaitForSingleObject(process.as_raw(), INFINITE) };
            if waited != WAIT_OBJECT_0 {
                record_failure(&mut failure, last("WaitForSingleObject(cleanup)"));
            }
        }
        if self.process.is_none() && !self.birth_phase.confirmed_unborn() {
            record_failure(
                &mut failure,
                ChildError::Unsupported("cleanup leader missing"),
            );
        }
        if terminated && let Some(active_job) = self.job.as_ref() {
            // TerminateJobObject 的返回不证明所有普通后代已退出。固定结构逐次查询；
            // 只在有限观察期内等待，避免无法确认 Job 终态时无限循环。
            let observed = observe_job_empty(Duration::from_secs(1), || {
                let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                #[cfg(test)]
                let queried = super::windows_cleanup_hooks::WindowsCleanupHooks::query(|| unsafe {
                    QueryInformationJobObject(
                        active_job.as_raw(),
                        JobObjectBasicAccountingInformation,
                        (&raw mut accounting).cast(),
                        std::mem::size_of_val(&accounting) as u32,
                        null_mut(),
                    )
                });
                #[cfg(not(test))]
                let queried = unsafe {
                    QueryInformationJobObject(
                        active_job.as_raw(),
                        JobObjectBasicAccountingInformation,
                        (&raw mut accounting).cast(),
                        std::mem::size_of_val(&accounting) as u32,
                        null_mut(),
                    )
                };
                if queried == 0 {
                    return Err(last("QueryInformationJobObject(cleanup)"));
                }
                Ok(accounting.ActiveProcesses)
            });
            if let Err(error) = observed {
                record_failure(&mut failure, error);
            }
        }
        if let Some(stdout) = self.stdout.as_mut()
            && let Err(error) = stdout.cancel_pending()
        {
            record_failure(&mut failure, error);
        }
        if let Some(stderr) = self.stderr.as_mut()
            && let Err(error) = stderr.cancel_pending()
        {
            record_failure(&mut failure, error);
        }
        if let Some(control) = self.control.as_mut()
            && let Err(error) = control.cleanup()
        {
            record_failure(&mut failure, error);
        }
        if !self.birth_phase.confirmed_unborn() && (self.stdout.is_none() || self.stderr.is_none())
        {
            record_failure(
                &mut failure,
                ChildError::Unsupported("born pipe owner missing"),
            );
        }
        if let Some(error) = failure {
            return Err(error);
        }
        // 只有真实 leader wait、Job 零活动及所有管道完成后才能释放责任并记为完成。
        self.process.take();
        self.job.take();
        self.cleaned = true;
        Ok(())
    }
    /// 参数：无；返回：依次为原 Job 是否持有、原 leader 是否持有、是否已确认清理完成。

    /// 测试只读快照：原Job、leader责任句柄是否仍持有，以及是否已宣称清理完成。
    #[cfg(test)]
    pub(super) fn cleanup_state_for_test(&self) -> (bool, bool, bool) {
        (self.job.is_some(), self.process.is_some(), self.cleaned)
    }
}

impl Drop for WindowsChild {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn pipe_name(name: &str) -> Vec<u16> {
    format!(r"\\.\pipe\{name}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

fn last(context: &'static str) -> ChildError {
    ChildError::io(context, io::Error::last_os_error())
}

fn record_failure(current: &mut Option<ChildError>, error: ChildError) {
    *current = Some(match current.take() {
        Some(primary) => primary.with_cleanup(Err(error)),
        None => error,
    });
}

fn observe_job_empty<F>(limit: Duration, mut active_processes: F) -> Result<(), ChildError>
where
    F: FnMut() -> Result<u32, ChildError>,
{
    let observed_at = Instant::now();
    loop {
        if active_processes()? == 0 {
            return Ok(());
        }
        if observed_at.elapsed() >= limit {
            return Err(ChildError::Io(format!(
                "QueryInformationJobObject(cleanup): ActiveProcesses remained nonzero after {} ms",
                limit.as_millis()
            )));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
fn duplicate_handle_for_test(
    raw: HANDLE,
    context: &'static str,
) -> Result<OwnedHandle, ChildError> {
    let process = unsafe { GetCurrentProcess() };
    let mut copy = null_mut();
    if unsafe {
        DuplicateHandle(
            process,
            raw,
            process,
            &mut copy,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(last(context));
    }
    OwnedHandle::from_raw(copy, context)
}

#[cfg(test)]
mod tests {
    use super::observe_job_empty;
    use crate::native_child::ChildError;
    use std::time::{Duration, Instant};

    #[test]
    fn injected_nonzero_accounting_stops_at_observation_limit() {
        let started = Instant::now();
        let error = observe_job_empty(Duration::from_millis(25), || Ok(1)).unwrap_err();
        assert!(
            matches!(error, ChildError::Io(message) if message.contains("ActiveProcesses remained nonzero"))
        );
        assert!(started.elapsed() >= Duration::from_millis(25));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn injected_accounting_query_error_fails_without_retrying_as_success() {
        let error = observe_job_empty(Duration::from_secs(1), || {
            Err(ChildError::Io("injected accounting failure".to_owned()))
        })
        .unwrap_err();
        assert!(
            matches!(error, ChildError::Io(message) if message == "injected accounting failure")
        );
    }
}
