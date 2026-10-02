//! Win10+ 原生 CreateProcessW：进程在运行前即绑定本任务 Job。

use std::io;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
#[cfg(test)]
use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle};
use windows_sys::Win32::Foundation::{GENERIC_READ, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};
#[cfg(test)]
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, PROCESS_INFORMATION, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, WaitForSingleObject,
};

use super::super::probe_budget::ProbeBudget;
use super::super::probe_failure::ProbeFailure;
use super::attribute_list::AttributeList;
use super::overlapped_pipe::OverlappedPipe;
use super::owned_handle::OwnedHandle;
use super::pipe_security::PipeSecurity;
use super::windows_command_line::WindowsCommandLine;

/// 本次 Windows 探针的 Job、leader 及两条自有管道。来源：Win32 CreateProcessW、Job Objects 与 Overlapped I/O。
pub(in crate::live_evidence) struct WindowsProbeChild {
    job: Option<OwnedHandle>,
    process: Option<OwnedHandle>,
    stdout: OverlappedPipe,
    stderr: OverlappedPipe,
    exit: Option<i32>,
    cleaned: bool,
}

impl WindowsProbeChild {
    /// 创建被独立 Job 原子约束的 suspended 子进程。
    /// 参数：command 是显式环境命令，budget 是共享期限与输出预算。返回：受 Job 约束的子进程所有权或失败。
    pub(in crate::live_evidence) fn spawn(
        command: &mut Command,
        budget: &mut ProbeBudget,
    ) -> Result<Self, ProbeFailure> {
        budget.check()?;
        let mut input = WindowsCommandLine::from_command(command)?;
        let security = PipeSecurity::for_current_user()?;
        let nonce = uuid::Uuid::new_v4();
        let stdout_name = pipe_name(&format!("diskgraph-probe-{nonce}-out"));
        let stderr_name = pipe_name(&format!("diskgraph-probe-{nonce}-err"));
        let (stdout, stdout_writer) = OverlappedPipe::create(&stdout_name, &security)?;
        let (stderr, stderr_writer) = OverlappedPipe::create(&stderr_name, &security)?;
        let mut inheritable = security.attributes(true);
        inheritable.lpSecurityDescriptor = null_mut();
        let nul: Vec<u16> = r"\\.\NUL"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let stdin = OwnedHandle::from_raw(
            unsafe {
                CreateFileW(
                    nul.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &inheritable,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                )
            },
            "CreateFileW(NUL stdin)",
        )?;
        let job = OwnedHandle::from_raw(
            unsafe { CreateJobObjectW(null(), null()) },
            "CreateJobObjectW",
        )?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.as_raw(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(last("SetInformationJobObject(KILL_ON_JOB_CLOSE)"));
        }
        let mut attributes = AttributeList::new()?;
        let handles: [HANDLE; 3] = [
            stdin.as_raw(),
            stdout_writer.as_raw(),
            stderr_writer.as_raw(),
        ];
        attributes.set_jobs([job.as_raw()])?;
        attributes.set_handles(handles)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = attributes.as_raw();
        let mut info = PROCESS_INFORMATION::default();
        budget.check()?;
        let application = input.application();
        let arguments = input.arguments();
        let environment = input.environment();
        let directory = input.directory();
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
            return Err(last("CreateProcessW(JOB_LIST,HANDLE_LIST)"));
        }
        let process = OwnedHandle::from_raw(info.hProcess, "CreateProcessW(process handle)")?;
        let thread = OwnedHandle::from_raw(info.hThread, "CreateProcessW(thread handle)")?;
        let mut child = Self {
            job: Some(job),
            process: Some(process),
            stdout,
            stderr,
            exit: None,
            cleaned: false,
        };
        // Child 已有且 suspended；父本地写端必须立即关闭，避免 EOF 被自身阻止。
        drop(stdin);
        drop(stdout_writer);
        drop(stderr_writer);
        #[cfg(test)]
        if command.get_envs().any(|(key, value)| {
            key.to_string_lossy()
                .eq_ignore_ascii_case("DG_WINDOWS_NATIVE_FAULT")
                && value.is_some_and(|value| value.to_string_lossy() == "post_create")
        }) {
            // 线程句柄也是进程引用，必须在 accounting 观察前释放。
            drop(thread);
            let error = ProbeFailure::Unsupported("injected post-create failure");
            return Err(error.with_cleanup(child.cleanup()));
        }
        if let Err(error) = budget.check() {
            drop(thread);
            return Err(error.with_cleanup(child.cleanup()));
        }
        let previous = unsafe { ResumeThread(thread.as_raw()) };
        let resume_failure = if previous == u32::MAX {
            Some(last("ResumeThread"))
        } else if previous != 1 {
            Some(ProbeFailure::Unsupported(
                "probe thread did not have exactly one suspend count",
            ))
        } else {
            None
        };
        drop(thread);
        if let Some(error) = resume_failure {
            return Err(error.with_cleanup(child.cleanup()));
        }
        if let Err(error) = budget.check() {
            return Err(error.with_cleanup(child.cleanup()));
        }
        Ok(child)
    }

    /// 观察 leader 是否已退出。参数：无。返回：true 表示已记录退出码，错误表示 Win32 查询失败。
    pub(in crate::live_evidence) fn poll(&mut self) -> Result<bool, ProbeFailure> {
        if self.exit.is_some() {
            return Ok(true);
        }
        let process = self
            .process
            .as_ref()
            .ok_or(ProbeFailure::Unsupported("probe leader already cleaned"))?;
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
    pub(in crate::live_evidence) fn exit_code(&self) -> Option<i32> {
        self.exit
    }

    /// 非阻塞读取 stdout 的固定片段。参数：无。返回：有数据时为最多 4096 字节，否则为 None 或错误。
    pub(in crate::live_evidence) fn read_stdout(&mut self) -> Result<Option<&[u8]>, ProbeFailure> {
        self.stdout.read_next()
    }

    /// 非阻塞读取 stderr 的固定片段。参数：无。返回：有数据时为最多 4096 字节，否则为 None 或错误。
    pub(in crate::live_evidence) fn read_stderr(&mut self) -> Result<Option<&[u8]>, ProbeFailure> {
        self.stderr.read_next()
    }

    /// 查询 stdout 是否已确认 EOF。参数：无。返回：确认 EOF 时为 true。
    pub(in crate::live_evidence) fn stdout_eof(&self) -> bool {
        self.stdout.eof()
    }

    /// 查询 stderr 是否已确认 EOF。参数：无。返回：确认 EOF 时为 true。
    pub(in crate::live_evidence) fn stderr_eof(&self) -> bool {
        self.stderr.eof()
    }

    /// 为 Windows 原生验收复制本 Job 句柄，便于清理前后直接核验活动进程数。
    /// 参数：无。返回：测试独有的 Job 句柄所有权或 Win32 错误。
    #[cfg(test)]
    pub(super) fn duplicate_job_for_test(&self) -> Result<OwnedHandle, ProbeFailure> {
        let job = self
            .job
            .as_ref()
            .ok_or(ProbeFailure::Unsupported("probe Job already cleaned"))?;
        duplicate_handle_for_test(job.as_raw(), "DuplicateHandle(test Job)")
    }

    /// 为 Windows 原生验收复制 leader 句柄，模拟外部宿主保持已退出进程引用。
    /// 参数：无。返回：测试独有的进程句柄所有权或 Win32 错误。
    #[cfg(test)]
    pub(super) fn duplicate_leader_for_test(&self) -> Result<OwnedHandle, ProbeFailure> {
        let process = self
            .process
            .as_ref()
            .ok_or(ProbeFailure::Unsupported("probe leader already cleaned"))?;
        duplicate_handle_for_test(process.as_raw(), "DuplicateHandle(test leader)")
    }

    /// 清理本次 Job 与所有普通后代并回收 pending I/O。
    /// 参数：无。返回：Job 活动进程归零且管道回收成功，或可包含多项原因的清理错误。
    /// Job 归零只观察一秒；leader 等待与 pending I/O 安全回收仍可能超过协作期限。
    pub(in crate::live_evidence) fn cleanup(&mut self) -> Result<(), ProbeFailure> {
        if self.cleaned {
            return Ok(());
        }
        self.cleaned = true;
        let mut failure = None;
        let mut job = self.job.take();
        let terminated = if let Some(active_job) = job.as_ref() {
            if unsafe { TerminateJobObject(active_job.as_raw(), 1) } == 0 {
                record_failure(&mut failure, last("TerminateJobObject"));
                // 终止请求失败时先关闭 Job，触发 KILL_ON_JOB_CLOSE 后备。
                drop(job.take());
                false
            } else {
                true
            }
        } else {
            false
        };
        // 先关闭 leader 句柄，避免它作为进程引用妨碍 ActiveProcesses 归零。
        if let Some(process) = self.process.take() {
            if unsafe { WaitForSingleObject(process.as_raw(), INFINITE) } != WAIT_OBJECT_0 {
                record_failure(&mut failure, last("WaitForSingleObject(cleanup)"));
            }
            drop(process);
        }
        if terminated && let Some(active_job) = job.as_ref() {
            // TerminateJobObject 的返回不证明所有普通后代已退出。固定结构逐次查询；
            // 外部进程句柄可延迟计数归零，所以只在有限观察期内等待。
            let observed_at = Instant::now();
            loop {
                let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                if unsafe {
                    QueryInformationJobObject(
                        active_job.as_raw(),
                        JobObjectBasicAccountingInformation,
                        (&raw mut accounting).cast(),
                        std::mem::size_of_val(&accounting) as u32,
                        null_mut(),
                    )
                } == 0
                {
                    record_failure(&mut failure, last("QueryInformationJobObject(cleanup)"));
                    break;
                }
                if accounting.ActiveProcesses == 0 {
                    break;
                }
                if observed_at.elapsed() >= Duration::from_secs(1) {
                    record_failure(
                        &mut failure,
                        ProbeFailure::Io(
                            "QueryInformationJobObject(cleanup): ActiveProcesses remained nonzero after 1 s"
                                .to_owned(),
                        ),
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        // 查询失败时仍关闭 Job 触发后备终止，但明确返回不完整清理错误。
        drop(job);
        if let Err(error) = self.stdout.cancel_pending() {
            record_failure(&mut failure, error);
        }
        if let Err(error) = self.stderr.cancel_pending() {
            record_failure(&mut failure, error);
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for WindowsProbeChild {
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

fn last(context: &'static str) -> ProbeFailure {
    ProbeFailure::io(context, io::Error::last_os_error())
}

fn record_failure(current: &mut Option<ProbeFailure>, error: ProbeFailure) {
    *current = Some(match current.take() {
        Some(primary) => primary.with_cleanup(Err(error)),
        None => error,
    });
}

#[cfg(test)]
fn duplicate_handle_for_test(
    raw: HANDLE,
    context: &'static str,
) -> Result<OwnedHandle, ProbeFailure> {
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
