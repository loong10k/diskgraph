use super::ChildError;
use super::linux_scanner_filter::LinuxScannerFilter;
use super::linux_scanner_receipt::LinuxScannerReceipt;
use std::ffi::OsStr;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

/// 只为固定 helper 创建 fresh Command，预构建 BPF/FD 后使用 syscall-only pre_exec。
/// 来源：原生 Rust OS-child 的私有 session 与 Linux seccomp/SCM_RIGHTS 启动闭包。
pub(super) struct LinuxScannerCommand {
    command: Command,
    receipt: LinuxScannerReceipt,
}

impl LinuxScannerCommand {
    /// 准备固定 scanner 启动。参数：program/args/environment 为受信宿主值；返回：无新权限的启动材料。
    pub(super) fn new(
        program: &Path,
        args: &[&OsStr],
        environment: &[(&OsStr, &OsStr)],
    ) -> Result<Self, ChildError> {
        let filter = LinuxScannerFilter::new()?;
        let (receipt, sender) = LinuxScannerReceipt::pair()?;
        let receiver_fd = receipt.receiver_fd();
        let mut command = Command::new(program);
        command.args(args).envs(environment.iter().copied());
        // 安全性：filter 与 sender 在 fork 前构造。child 只做已准备栈数据和纯 syscall，
        // 不分配 Rust/String/锁；私有 receipt 两端不会进入 exec 后的 helper。
        unsafe {
            command.pre_exec(move || {
                libc::close(receiver_fd);
                let pid = libc::getpid();
                if libc::setsid() != pid {
                    return Err(io::Error::last_os_error());
                }
                if libc::getpgrp() != pid || libc::getsid(0) != pid {
                    return Err(io::Error::from_raw_os_error(libc::EPERM));
                }
                filter.install()?;
                let result = LinuxScannerReceipt::send_self(sender.as_raw_fd(), pid);
                libc::close(sender.as_raw_fd());
                result
            });
        }
        Ok(Self { command, receipt })
    }

    /// 转移父启动材料。参数：无；返回：fresh Command 与唯一 receiver，调用方持实际 Child。
    pub(super) fn into_parts(self) -> (Command, LinuxScannerReceipt) {
        (self.command, self.receipt)
    }
}
