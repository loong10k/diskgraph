use super::linux_atomic_exit::LinuxAtomicExit;
use super::linux_atomic_pipes::LinuxAtomicPipes;
use super::unix_control_channel::UnixControlChannel;
use super::{ChildError, ChildSpawnError, ControlWriteStatus};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

/// 唯一原子 child owner，实际 pidfd、控制写端和两输出管道只随本对象转移。
/// 来源：原生 Rust Linux CLONE_PIDFD 与真实非阻塞管道生命周期。
pub(crate) struct LinuxAtomicChild {
    exit: LinuxAtomicExit,
    pipes: LinuxAtomicPipes,
    control: Option<UnixControlChannel>,
    control_closed: bool,
    stdout_eof: bool,
    stderr_eof: bool,
    normal_complete: bool,
}

impl LinuxAtomicChild {
    /// 参数：exit、pipes 为原子出生材料；返回：无分配/检查点的 owner，尚未初始化父端 I/O。
    pub(super) fn adopt(exit: LinuxAtomicExit, pipes: LinuxAtomicPipes) -> Self {
        Self {
            exit,
            pipes,
            control: None,
            control_closed: false,
            stdout_eof: false,
            stderr_eof: false,
            normal_complete: false,
        }
    }

    /// 参数：无；返回：在唯一 owner 已建立后关闭父 child 副本并设置真实非阻塞端点。
    pub(super) fn configure(&mut self) -> Result<(), ChildError> {
        self.pipes.close_child_copies();
        self.pipes.configure()?;
        let stream = self
            .pipes
            .control
            .take()
            .ok_or(ChildError::Unsupported("atomic control owner missing"))?;
        self.control = Some(UnixControlChannel::from_stream(stream)?);
        Ok(())
    }

    /// 参数：无；返回：借用有界启动通道 fd，不授予镜像或发布信任。
    pub(super) fn startup_fd(&self) -> Result<i32, ChildError> {
        self.pipes
            .startup
            .as_ref()
            .map(AsRawFd::as_raw_fd)
            .ok_or(ChildError::Unsupported("atomic startup channel closed"))
    }

    /// 参数：无；返回：释放启动通道；EOF 不是执行成功或物理退出许可。
    pub(super) fn finish_startup(&mut self) {
        self.pipes.startup.take();
    }

    /// 参数：bytes 为非空最多4096字节；返回：真实 Written/Pending 或原 I/O 错误。
    pub(crate) fn start_control_write(
        &mut self,
        bytes: &[u8],
    ) -> Result<ControlWriteStatus, ChildError> {
        self.control
            .as_mut()
            .ok_or(ChildError::Unsupported("atomic control input closed"))?
            .start_write(bytes)
    }

    /// 参数：无；返回：同一 owned chunk 的真实进度；不得重复发送已完成片段。
    pub(crate) fn poll_control_write(&mut self) -> Result<ControlWriteStatus, ChildError> {
        if self.control_closed {
            return Ok(ControlWriteStatus::Closed);
        }
        self.control
            .as_mut()
            .ok_or(ChildError::Unsupported("atomic control not initialized"))?
            .poll_write()
    }

    /// 参数：无；返回：实际关闭父写端；不保证未发送协议帧完整。
    pub(crate) fn request_control_close(&mut self) -> Result<ControlWriteStatus, ChildError> {
        self.control_closed = true;
        if let Some(mut control) = self.control.take() {
            control.close();
        }
        self.pipes.control.take();
        Ok(ControlWriteStatus::Closed)
    }

    /// 参数：无；返回：一笔至多4096字节的借用 stdout，Pending/EOF 均返回 None。
    pub(crate) fn read_stdout(&mut self) -> Result<Option<&[u8]>, ChildError> {
        read_once(
            &mut self.pipes.stdout,
            &mut self.stdout_eof,
            self.pipes.buffer.bytes_mut(),
            "read atomic stdout",
        )
    }

    /// 参数：无；返回：一笔至多4096字节的借用 stderr，调用方公平排空两端。
    pub(crate) fn read_stderr(&mut self) -> Result<Option<&[u8]>, ChildError> {
        read_once(
            &mut self.pipes.stderr,
            &mut self.stderr_eof,
            self.pipes.buffer.bytes_mut(),
            "read atomic stderr",
        )
    }

    /// 参数：无；返回：真实整个原线程组 readiness，不回收、不以 main Z 代替。
    pub(crate) fn poll(&mut self) -> Result<bool, ChildError> {
        self.exit.ready()
    }

    /// 参数：checkpoint 为调用方原期限/授权检查；返回：整线程组退出、两EOF、控制Closed及实际wait。
    pub(crate) fn poll_normal_exit<E>(
        &mut self,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<bool, ChildSpawnError<E>> {
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        let complete = if self.normal_complete {
            true
        } else if self.exit.reaped() {
            return Err(ChildError::Unsupported(
                "atomic child was reaped outside normal completion",
            )
            .into());
        } else if !self.control_closed
            || !self.stdout_eof
            || !self.stderr_eof
            || !self.exit.ready()?
        {
            false
        } else {
            self.exit.reap_normal()?;
            self.normal_complete = true;
            true
        };
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        Ok(complete)
    }

    /// 参数：无；返回：正常真实退出码，不把信号失败伪造为0。
    pub(crate) fn exit_code(&self) -> Option<i32> {
        self.exit.exit_code()
    }

    /// 参数：无；返回：实际终止信号。
    pub(crate) fn exit_signal(&self) -> Option<i32> {
        self.exit.exit_signal()
    }

    /// 参数：无；返回：stdout 实际读取到 EOF 的事实。
    pub(crate) fn stdout_eof(&self) -> bool {
        self.stdout_eof
    }

    /// 参数：无；返回：stderr 实际读取到 EOF 的事实。
    pub(crate) fn stderr_eof(&self) -> bool {
        self.stderr_eof
    }

    /// 参数：无；返回：原 pidfd 停止并实际wait；不持DB/registry锁，不依赖数字身份。
    pub(crate) fn cleanup(&mut self) -> Result<(), ChildError> {
        self.request_control_close()?;
        self.finish_startup();
        self.pipes.close_child_copies();
        self.exit.cleanup()
    }

    /// 参数：无；返回：原 pidfd 的物理退场事实，不等同原 wait 已消费或正常许可。
    pub(crate) fn physically_exited(&self) -> Result<bool, ChildError> {
        self.exit.stopped()
    }

    /// 参数：无；返回：本owner成功消费原 P_PIDFD wait 的事实；外部reap不能伪造此值。
    pub(crate) fn reaped(&self) -> bool {
        self.exit.reaped()
    }

    /// 参数：无；返回：原子 pidfd 的测试借用，不重新打开进程身份。
    #[cfg(test)]
    pub(super) fn pidfd_for_test(&self) -> i32 {
        self.exit.fd()
    }

    /// 参数：无；返回：测试观察 /proc 原对象使用的 PID，不作 signal fallback。
    #[cfg(test)]
    pub(super) fn pid_for_test(&self) -> i32 {
        self.exit.pid()
    }

    /// 参数：无；返回：本案实际关闭 Init/Ready 通道，用于真实 EOF 自然退出测试。
    #[cfg(test)]
    pub(super) fn close_startup_for_test(&mut self) {
        self.finish_startup();
    }
}

impl std::fmt::Debug for LinuxAtomicChild {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("LinuxAtomicChild")
            .field("reaped", &self.exit.reaped())
            .field("stdout_eof", &self.stdout_eof)
            .field("stderr_eof", &self.stderr_eof)
            .finish()
    }
}

impl Drop for LinuxAtomicChild {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _ = writeln!(
                std::io::stderr(),
                "atomic child Drop cleanup failed: {error}"
            );
        }
    }
}

fn read_once<'a>(
    pipe: &mut Option<std::fs::File>,
    eof: &mut bool,
    buffer: &'a mut [u8],
    context: &str,
) -> Result<Option<&'a [u8]>, ChildError> {
    let Some(pipe) = pipe.as_mut() else {
        return Ok(None);
    };
    match pipe.read(buffer) {
        Ok(0) => {
            *eof = true;
            Ok(None)
        }
        Ok(count) => Ok(Some(&buffer[..count])),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(ChildError::io(context, error)),
    }
}
