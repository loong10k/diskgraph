use super::linux_scanner_command::LinuxScannerCommand;
use super::linux_scanner_exit::LinuxScannerExit;
use super::linux_scanner_receipt::LinuxScannerReceipt;
use super::unix_child_setup::UnixChildSetup;
use super::unix_control_channel::UnixControlChannel;
use super::unix_normal_exit::UnixNormalExit;
use super::{ChildError, ChildSpawnError, UnixChild};
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Child, Stdio};
use std::thread::{self, JoinHandle};

/// 持有 exec 前握手与实际启动线程；接收失败先拒绝 exec，再真实 join 回收。
/// 来源：原生 Rust Command::spawn 的 pre_exec 错误回收与 Linux 私有 pidfd ACK 通道。
pub(super) struct LinuxScannerLaunch {
    receipt: Option<LinuxScannerReceipt>,
    launcher: Option<JoinHandle<io::Result<Child>>>,
    child: Option<Child>,
    exit: Option<LinuxScannerExit>,
    allowed: bool,
}

impl LinuxScannerLaunch {
    /// 创建固定受限 scanner owner，不把接收失败降级为已执行但无人回收的 child。
    /// 参数：program/args/environment 为受信宿主值，checkpoint 借用原请求检查。
    /// 返回：同一原 pidfd 与实际 Child 的唯一 owner，或原错误及实际清理错误。
    pub(super) fn spawn<E>(
        program: &Path,
        args: &[&OsStr],
        environment: &[(&OsStr, &OsStr)],
        mut checkpoint: impl FnMut() -> Result<(), E>,
    ) -> Result<UnixChild, ChildSpawnError<E>> {
        checkpoint().map_err(ChildSpawnError::checkpoint)?;
        UnixChildSetup::reject_auto_reap()?;
        let (mut command, receipt) =
            LinuxScannerCommand::new(program, args, environment)?.into_parts();
        let (control, stdin) = UnixControlChannel::pair()?;
        command
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let launcher = thread::Builder::new()
            .name("scanner-pre-exec".into())
            .spawn(move || {
                let result = command.spawn();
                // 与旧 A 相同，成功/失败都关闭 Command 留存的 child stdin 副本。
                command.stdin(Stdio::null());
                result
            })
            .map_err(|error| ChildError::io("start scanner launcher", error))?;
        let mut launch = Self {
            receipt: Some(receipt),
            launcher: Some(launcher),
            child: None,
            exit: None,
            allowed: false,
        };
        let received: Result<(), ChildSpawnError<E>> = (|| {
            loop {
                match launch
                    .receipt
                    .as_ref()
                    .ok_or(ChildError::Unsupported("scanner receipt owner lost"))?
                    .receive()
                {
                    Ok(Some(exit)) => {
                        launch.exit = Some(exit);
                        break;
                    }
                    Ok(None) => {
                        // 此等待不是新预算；若原请求拒绝，关闭通道使 pre_exec 返回原 spawn 错误。
                        if !launch
                            .receipt
                            .as_ref()
                            .ok_or(ChildError::Unsupported("scanner receipt owner lost"))?
                            .wait_readable()?
                        {
                            checkpoint().map_err(ChildSpawnError::checkpoint)?;
                        }
                    }
                    Err(error) => {
                        // 安装失败先表现为通道 EOF；实际 spawn 的原 OS 错误优先于传输后验。
                        launch
                            .receipt
                            .as_ref()
                            .ok_or(ChildError::Unsupported("scanner receipt owner lost"))?
                            .cancel()?;
                        return match launch.join()? {
                            Err(error) => Err(ChildError::io("spawn", error).into()),
                            Ok(child) => {
                                launch.child = Some(child);
                                Err(error.into())
                            }
                        };
                    }
                }
            }
            checkpoint().map_err(ChildSpawnError::checkpoint)?;
            launch
                .receipt
                .as_ref()
                .ok_or(ChildError::Unsupported("scanner receipt owner lost"))?
                .allow_exec()?;
            launch.allowed = true;
            Ok(())
        })();
        if let Err(error) = received {
            return Err(error.with_cleanup(launch.abort()));
        }
        let child = match launch.join()? {
            Ok(child) => child,
            Err(error) => return Err(ChildError::io("spawn", error).into()),
        };
        launch.receipt.take();
        let exit = launch
            .exit
            .take()
            .ok_or(ChildError::Unsupported("scanner pidfd owner lost"))?;
        if exit.pid() != child.id() {
            return Err(
                ChildError::Unsupported("scanner Child and pidfd identities differ")
                    .with_cleanup(exit.terminate_and_reap().map(|_| ()))
                    .into(),
            );
        }
        UnixChild::from_spawn(
            child,
            Some(control),
            false,
            UnixNormalExit::LinuxScanner(exit),
            false,
            checkpoint,
        )
    }

    fn join(&mut self) -> Result<io::Result<Child>, ChildError> {
        let launcher = self
            .launcher
            .take()
            .ok_or(ChildError::Unsupported("scanner launcher already joined"))?;
        match launcher.join() {
            Ok(result) => Ok(result),
            Err(panic) => {
                if let Some(receipt) = &self.receipt {
                    let _ = receipt.cancel();
                }
                if let Some(exit) = &self.exit {
                    let _ = exit.terminate_and_reap();
                }
                std::panic::resume_unwind(panic)
            }
        }
    }

    fn abort(&mut self) -> Result<(), ChildError> {
        // 先关闭 ACK 通道。未获准的 child 不可能进入 helper；std spawn 会回收 pre_exec 失败。
        // shutdown 交付 EOF，但 fd 在 join 前不释放，避免尚未 fork 的 pre_exec 关闭复用 fd。
        let closed = match &self.receipt {
            Some(receipt) => receipt.cancel(),
            None => Ok(()),
        };
        let stopped = if self.allowed {
            self.exit
                .as_ref()
                .ok_or(ChildError::Unsupported("allowed scanner has no pidfd"))?
                .terminate()
        } else {
            Ok(())
        };
        if self.launcher.is_some() {
            if let Ok(child) = self.join()? {
                self.child = Some(child);
            }
        }
        self.receipt.take();
        if self.child.is_none() {
            // join 已返回 spawn 错误时 std 已消费 child，不再对旧数值身份执行任何操作。
            return closed.and(stopped);
        }
        let reaped = match &self.exit {
            Some(exit) => exit.terminate_and_reap().map(|_| ()),
            None => Err(ChildError::Unsupported(
                "unacknowledged child exited without a pidfd receipt; reliable reap unavailable",
            )),
        };
        if reaped.is_ok() {
            self.child.take();
        }
        closed
            .and(stopped)
            .map_err(|error| error.with_cleanup(reaped.clone()))?;
        reaped
    }
}

impl Drop for LinuxScannerLaunch {
    fn drop(&mut self) {
        if self.launcher.is_some() || self.child.is_some() {
            let _ = self.abort();
        }
    }
}
