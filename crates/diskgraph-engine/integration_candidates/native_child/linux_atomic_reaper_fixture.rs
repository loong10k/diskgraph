//! 在线程局部 waitid 拒绝前创建真实回收线程，不复刻生产出生算法。

use super::ChildError;
use super::linux_atomic_child::LinuxAtomicChild;
use super::linux_atomic_launcher_test_support::{assert_reaped, duplicate};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{JoinHandle, spawn};

type ReapMaterial = (Option<LinuxAtomicChild>, Option<OwnedFd>);
type ReapResult = (bool, Result<(), ChildError>, bool, io::Result<()>);

/// 测试专用有限回收线程，接收原 owner 并消费其真实 P_PIDFD wait。
/// 来源：原生 Rust 测试宿主线程隔离与 Linux 原 pidfd 生命周期，非产品 reaper。
pub(super) struct AtomicReaperFixture {
    sender: Option<SyncSender<ReapMaterial>>,
    thread: Option<JoinHandle<ReapResult>>,
    witness: Option<OwnedFd>,
}

impl AtomicReaperFixture {
    /// 参数：无；返回：先于当前线程 seccomp 建立的未过滤真实线程。
    pub(super) fn new() -> Self {
        let (sender, receiver) = sync_channel::<ReapMaterial>(1);
        let thread = spawn(move || {
            let Ok((owner, witness)) = receiver.recv() else {
                return (false, Ok(()), false, Ok(()));
            };
            let present = owner.is_some();
            let (cleanup, reaped) = if let Some(mut child) = owner {
                let cleanup = child.cleanup();
                let reaped = child.reaped();
                drop(child);
                (cleanup, reaped)
            } else {
                (Ok(()), false)
            };
            // 若候选错误地丢失 owner，测试仍用原 duplicate pidfd 回收，再由外层断言判失败。
            // 该救援只保护测试资源，绝不计为产品正常退出许可或产品 owner 成功。
            let rescued = if !reaped {
                witness.as_ref().map_or(Ok(()), reap_witness)
            } else {
                Ok(())
            };
            if rescued.is_ok()
                && let Some(witness) = witness.as_ref()
            {
                assert_reaped(witness);
            }
            (present, cleanup, reaped, rescued)
        });
        Self {
            sender: Some(sender),
            thread: Some(thread),
            witness: None,
        }
    }

    /// 参数：child 为本案实际出生 owner；返回：保存原 pidfd duplicate 供异常测试宿主兜底。
    pub(super) fn remember(&mut self, child: &LinuxAtomicChild) {
        self.witness = Some(duplicate(child));
    }

    /// 参数：owner 为失败或 unwind 返回的原对象；返回：完成真实回收后是否保留了原 owner。
    pub(super) fn finish(mut self, owner: Option<LinuxAtomicChild>) -> bool {
        self.sender
            .take()
            .unwrap()
            .send((owner, self.witness.take()))
            .unwrap();
        let (present, cleanup, reaped, rescue) = self.thread.take().unwrap().join().unwrap();
        rescue.unwrap();
        cleanup.unwrap();
        if present {
            assert!(reaped, "unfiltered thread consumed original owner wait");
        }
        present
    }
}

impl Drop for AtomicReaperFixture {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send((None, self.witness.take()));
        }
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok((_, cleanup, _, rescue)) => {
                    if let Err(error) = cleanup {
                        eprintln!("atomic test owner cleanup: {error}");
                    }
                    if let Err(error) = rescue {
                        eprintln!("atomic test witness rescue: {error}");
                    }
                }
                Err(_) => eprintln!("atomic test reaper unexpectedly panicked"),
            }
        }
    }
}

fn reap_witness(witness: &OwnedFd) -> io::Result<()> {
    if unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            witness.as_raw_fd(),
            libc::SIGKILL,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    } < 0
    {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    loop {
        if unsafe {
            libc::waitid(
                libc::P_PIDFD,
                witness.as_raw_fd() as u32,
                &mut info,
                libc::WEXITED,
            )
        } == 0
        {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ECHILD) {
            return Ok(());
        }
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
}
