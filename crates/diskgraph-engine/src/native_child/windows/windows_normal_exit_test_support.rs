//! Windows B 测试只读观察 owner；自然结束与失败救援严格分开。

use super::windows_normal_process_witness::WindowsNormalProcessWitness;
use std::cell::Cell;
use std::io;
use std::io::Read;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
    QueryInformationJobObject,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

use super::super::{ChildError, ChildInputMode, ChildSpawnError, ControlWriteStatus};
use super::owned_handle::OwnedHandle;
use super::windows_child::WindowsChild;
use super::windows_normal_exit_fixture::{command, end_marker};

/// 单请求真实 child 和独立只读 Job／leader 复制句柄。来源：Win32 Job accounting 与真实 wait。
pub(super) struct WindowsNormalExitTestSupport {
    pub(super) child: WindowsChild,
    directory: tempfile::TempDir,
    job: OwnedHandle,
    leader: OwnedHandle,
    deadline: Instant,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    diagnostic_samples: Cell<u8>,
    leader_identity: [u8; 12],
    descendant: Option<WindowsNormalProcessWitness>,
}

impl WindowsNormalExitTestSupport {
    /// 在一次原始期限内启动真实子入口。参数：mode 为夹具阶段。返回：唯一 owner 与保持存活的只读观察句柄。
    pub(super) fn spawn(mode: &str) -> Result<Self, ChildError> {
        let directory = tempfile::tempdir().map_err(|e| ChildError::io("normal fixture dir", e))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let child = crate::native_child::WindowsTestBirth::spawn_with_input(
            &mut command(mode, directory.path()),
            ChildInputMode::WorkerControl,
            || check(deadline),
        )
        .map_err(|e| match e {
            ChildSpawnError::Operation(e) => e,
            ChildSpawnError::Checkpoint { primary, cleanup } => {
                ChildError::io("normal fixture spawn", primary)
                    .with_cleanup(cleanup.map_or(Ok(()), Err))
            }
        })?;
        let job = child.duplicate_job_for_test()?;
        let leader = child.duplicate_leader_for_test()?;
        let leader_identity = WindowsNormalProcessWitness::record(leader.as_raw())?;
        Ok(Self {
            child,
            directory,
            job,
            leader,
            deadline,
            stdout: Vec::new(),
            stderr: Vec::new(),
            diagnostic_samples: Cell::new(0),
            leader_identity,
            descendant: None,
        })
    }

    /// 参数：无；在原期限内绑定出生记录的后代，读取最多13字节，损坏/查询失败不当成存活。
    pub(super) fn bind_descendant(&mut self) -> Result<(), ChildError> {
        loop {
            self.check()
                .map_err(|e| ChildError::io("normal target deadline", e))?;
            match std::fs::File::open(self.directory.path().join("descendant-identity")) {
                Ok(file) => {
                    let mut bytes = Vec::with_capacity(13);
                    file.take(13)
                        .read_to_end(&mut bytes)
                        .map_err(|e| ChildError::io("read normal descendant identity", e))?;
                    self.check()
                        .map_err(|e| ChildError::io("normal target deadline", e))?;
                    let identity: [u8; 12] = bytes.try_into().map_err(|_| {
                        ChildError::Unsupported("malformed normal descendant identity")
                    })?;
                    self.descendant = Some(WindowsNormalProcessWitness::open(
                        identity,
                        self.job.as_raw(),
                    )?);
                    self.check()
                        .map_err(|e| ChildError::io("normal target final deadline", e))?;
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(ChildError::io("open normal descendant identity", error)),
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 参数：无；返回原held leader身份/原Job成员与真实未退出的联合证据。
    pub(super) fn leader_alive(&self) -> Result<(), ChildError> {
        self.check()
            .map_err(|e| ChildError::io("normal leader deadline", e))?;
        WindowsNormalProcessWitness::verify_live(
            self.leader.as_raw(),
            &self.leader_identity,
            self.job.as_raw(),
        )?;
        self.check()
            .map_err(|e| ChildError::io("normal leader final deadline", e))
    }

    /// 参数：无；返回已经绑定的普通后代真实存活，不能以conhost或其它成员替代。
    pub(super) fn descendant_alive(&self) -> Result<(), ChildError> {
        self.check()
            .map_err(|e| ChildError::io("normal descendant deadline", e))?;
        self.descendant
            .as_ref()
            .ok_or(ChildError::Unsupported("normal descendant witness absent"))?
            .alive(self.job.as_raw())?;
        self.check()
            .map_err(|e| ChildError::io("normal descendant final deadline", e))
    }

    /// 借原期限执行 caller 检查。参数：无。返回：原 Instant 尚有效或精确超时。
    pub(super) fn check(&self) -> Result<(), io::Error> {
        check(self.deadline)
    }

    /// 返回同一个绝对期限，不重建时钟。参数：无。返回：spawn 前建立的期限。
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    /// 查询实际 Job 活动数，观察句柄不会触发关闭。参数：无。返回：Win32 当前活动进程数或原查询错误。
    pub(super) fn active(&self) -> Result<u32, ChildError> {
        let observed_at = Instant::now();
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        if unsafe {
            QueryInformationJobObject(
                self.job.as_raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(ChildError::io(
                "QueryInformationJobObject(normal test)",
                io::Error::last_os_error(),
            ));
        }
        // 最多四个非零观察点，避免原15秒轮询产生无界诊断；不改变返回值或原断言。
        if accounting.ActiveProcesses != 0 && self.diagnostic_samples.get() < 4 {
            self.diagnostic_samples
                .set(self.diagnostic_samples.get() + 1);
            super::windows_job_test_diagnostics::WindowsJobTestDiagnostics::observe(
                self.job.as_raw(),
                self.leader.as_raw(),
                accounting.ActiveProcesses,
                observed_at,
            );
        }
        Ok(accounting.ActiveProcesses)
    }

    /// 单次实际等待 leader 状态，不消费 Job 所有权。参数：无。返回：已退出或仍活动；其它 Win32 结果拒绝。
    pub(super) fn leader_exited(&self) -> Result<bool, ChildError> {
        match unsafe { WaitForSingleObject(self.leader.as_raw(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(ChildError::io(
                "WaitForSingleObject(normal test)",
                io::Error::last_os_error(),
            )),
        }
    }

    /// 从真实管道收取直到两条 EOF，并核对固定 End 字节。参数：无。返回：已有原始 End 标记和两个 OS EOF，或资格失败。
    pub(super) fn await_eof(&mut self) -> Result<(), ChildError> {
        loop {
            self.tick()?;
            if self.child.stdout_eof()
                && self.child.stderr_eof()
                && self.directory.path().join("standard-pipes-closed").exists()
            {
                let marker = end_marker();
                if !self
                    .stdout
                    .windows(marker.len())
                    .any(|bytes| bytes == marker)
                    || !self
                        .stderr
                        .windows(b"normal-stderr\n".len())
                        .any(|b| b == b"normal-stderr\n")
                {
                    return Err(ChildError::Unsupported(
                        "actual End/standard-close witnesses absent",
                    ));
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 待真实 leader 退出，原 child 同时记录退出码。参数：无。返回：保留的退出码或原期限失败。
    pub(super) fn await_leader_exit(&mut self) -> Result<i32, ChildError> {
        while !self.child.poll()? || !self.leader_exited()? {
            self.tick()?;
            std::thread::sleep(Duration::from_millis(1));
        }
        self.child
            .exit_code()
            .ok_or(ChildError::Unsupported("actual leader code absent"))
    }

    /// 观察 release 前 heartbeat 连续增加，不能仅以 ready 文件代替活动进程。参数：name 为本请求计数文件。返回：两个实际递增值或资格失败。
    pub(super) fn heartbeat_advances(&mut self, name: &str) -> Result<(u64, u64), ChildError> {
        let mut first = None;
        loop {
            self.tick()?;
            if let Ok(bytes) = std::fs::read(self.directory.path().join(name))
                && let Ok(bytes) = <[u8; 8]>::try_from(bytes.as_slice())
            {
                let sequence = u64::from_le_bytes(bytes);
                if let Some(previous) = first {
                    if sequence > previous {
                        return Ok((previous, sequence));
                    }
                } else {
                    first = Some(sequence);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 关闭逻辑控制输入但不把它当成 Job 退出。参数：无。返回：真实 Closed；其它状态是资格错误。
    pub(super) fn close_control(&mut self) -> Result<(), ChildError> {
        if self.child.request_control_close()? != ControlWriteStatus::Closed {
            return Err(ChildError::Unsupported("idle control did not close"));
        }
        Ok(())
    }

    /// 释放夹具并等待所有真实自然退出；不调用 TerminateJob 或 cleanup。参数：无。返回：leader/Job/EOF 全部已自然完成，或错误。
    pub(super) fn release_and_observe(&mut self) -> Result<(), ChildError> {
        std::fs::write(self.directory.path().join("release"), b"release")
            .map_err(|e| ChildError::io("normal fixture release", e))?;
        loop {
            self.tick()?;
            if self.child.poll()?
                && self.leader_exited()?
                && self.active()? == 0
                && self.child.stdout_eof()
                && self.child.stderr_eof()
            {
                if let Some(descendant) = &self.descendant {
                    descendant.exited_zero()?;
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 核对自然 release 标记，不代表 OS wait 已完成。参数：name 为 leader 或后代标记名。返回：标记实际存在。
    pub(super) fn natural_marker(&self, name: &str) -> bool {
        self.directory.path().join(name).exists()
    }

    fn tick(&mut self) -> Result<(), ChildError> {
        self.check()
            .map_err(|e| ChildError::io("normal fixture original deadline", e))?;
        if let Some(bytes) = self.child.read_stdout()? {
            self.stdout.extend_from_slice(bytes);
        }
        if let Some(bytes) = self.child.read_stderr()? {
            self.stderr.extend_from_slice(bytes);
        }
        if self.stdout.len() + self.stderr.len() > 64 * 1024 {
            return Err(ChildError::Unsupported(
                "normal fixture output exceeded bound",
            ));
        }
        Ok(())
    }
}

impl Drop for WindowsNormalExitTestSupport {
    fn drop(&mut self) {
        // 失败救援不计自然成功；观察句柄保留到被测 owner 的 cleanup 完成。
        let _ = std::fs::write(self.directory.path().join("release"), b"release");
        let _ = self.child.cleanup();
    }
}

/// 检查原绝对期限。参数：deadline 是 spawn 前一次捕获值。返回：仍有效或精确 TimedOut。
pub(super) fn check(deadline: Instant) -> Result<(), io::Error> {
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "original normal-exit test deadline",
        ))
    } else {
        Ok(())
    }
}
