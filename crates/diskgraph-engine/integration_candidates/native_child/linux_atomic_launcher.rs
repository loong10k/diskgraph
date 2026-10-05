use super::linux_atomic_abi::LinuxAtomicAbi;
use super::linux_atomic_child::LinuxAtomicChild;
use super::linux_atomic_exit::LinuxAtomicExit;
use super::linux_atomic_handshake::LinuxAtomicHandshake;
use super::linux_atomic_launch_failure::LinuxAtomicLaunchFailure;
use super::linux_atomic_pipes::LinuxAtomicPipes;
use super::linux_atomic_signal_mask::LinuxAtomicSignalMask;
use super::linux_scanner_filter::LinuxScannerFilter;
use super::unix_child_setup::UnixChildSetup;
use super::{ChildError, ChildSpawnError};
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::FileExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::Instant;

/// 从宿主 held ELF 和显式固定材料建立原子 child，不定位路径、不升级安装或请求权限。
/// 来源：原生 Rust/C CLONE_PIDFD 的父侧准备、原检查点和真实 pidfd 所有权。
pub(crate) struct LinuxAtomicLauncher {
    image: File,
    args: Vec<CString>,
    environment: Vec<CString>,
    filter: LinuxScannerFilter,
    pipes: LinuxAtomicPipes,
}

impl LinuxAtomicLauncher {
    /// 参数：image 为已打开的ELF，args/environment为受信宿主固定C字符串；返回：有限准备材料。
    /// 本方法只核对格式/架构，不建立摘要信任；不继承 PATH/装载环境，也不重新打开镜像。
    pub(crate) fn prepare(
        image: File,
        args: Vec<CString>,
        environment: Vec<CString>,
    ) -> Result<Self, ChildError> {
        validate_material(&image, &args, &environment)?;
        UnixChildSetup::reject_auto_reap()?;
        let filter = LinuxScannerFilter::new()?;
        let pipes = LinuxAtomicPipes::new(&image)?;
        Ok(Self {
            image,
            args,
            environment,
            filter,
            pipes,
        })
    }

    /// 参数：deadline 是原请求期限，checkpoint 借原账本/授权，unwind_owner是外部空处置槽。
    /// 返回：唯一owner或原错及必要处置owner；panic清理未消费时移交外槽并恢复原payload。
    /// Ready/启动EOF不证明成功exec、结果、物理退出或发布，调用方仍必须核验完整执行协议。
    pub(crate) fn spawn<E>(
        self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
        unwind_owner: &mut Option<LinuxAtomicChild>,
    ) -> Result<LinuxAtomicChild, LinuxAtomicLaunchFailure<E>> {
        self.spawn_inner(deadline, checkpoint, unwind_owner, None)
    }

    /// 参数：deadline/checkpoint/unwind_owner同生产，observer仅本请求出生后观察。
    /// 返回：同生产入口真实启动结果，不能替换算法。
    #[cfg(test)]
    pub(super) fn spawn_observed<E>(
        self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
        unwind_owner: &mut Option<LinuxAtomicChild>,
        observer: &mut impl FnMut(&mut LinuxAtomicChild) -> Result<(), ChildError>,
    ) -> Result<LinuxAtomicChild, LinuxAtomicLaunchFailure<E>> {
        self.spawn_inner(deadline, checkpoint, unwind_owner, Some(observer))
    }

    fn spawn_inner<E>(
        self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
        unwind_owner: &mut Option<LinuxAtomicChild>,
        observer: Option<&mut dyn FnMut(&mut LinuxAtomicChild) -> Result<(), ChildError>>,
    ) -> Result<LinuxAtomicChild, LinuxAtomicLaunchFailure<E>> {
        if unwind_owner.is_some() {
            return Err(LinuxAtomicLaunchFailure::before_birth(
                ChildError::Unsupported("atomic unwind owner slot is occupied").into(),
            ));
        }
        checkpoint().map_err(|primary| {
            LinuxAtomicLaunchFailure::before_birth(ChildSpawnError::checkpoint(primary))
        })?;
        if Instant::now() >= deadline {
            return Err(LinuxAtomicLaunchFailure::before_birth(
                ChildError::io(
                    "atomic birth original deadline",
                    io::Error::new(io::ErrorKind::TimedOut, "original deadline elapsed"),
                )
                .into(),
            ));
        }
        #[cfg(not(all(
            any(target_arch = "x86_64", target_arch = "aarch64"),
            target_endian = "little",
            target_pointer_width = "64"
        )))]
        {
            let _ = observer;
            Err(LinuxAtomicLaunchFailure::before_birth(
                ChildError::Unsupported("atomic birth syscall ABI is unsupported").into(),
            ))
        }
        #[cfg(all(
            any(target_arch = "x86_64", target_arch = "aarch64"),
            target_endian = "little",
            target_pointer_width = "64"
        ))]
        {
            // 全部指针数组与filter早于出生；child使用自己的COW内存，永不返回Rust调用栈。
            let argv: Vec<_> = self
                .args
                .iter()
                .map(|argument| argument.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            let environment: Vec<_> = self
                .environment
                .iter()
                .map(|argument| argument.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            let filter = self.filter.program();
            let request = LinuxAtomicAbi {
                fds: self.pipes.child_fds(),
                argv: argv.as_ptr(),
                environment: environment.as_ptr(),
                filter_count: filter.len,
                filter: filter.filter,
            };
            let mut mask = LinuxAtomicSignalMask::new();
            let mut pidfd = -1;
            let pid = unsafe {
                super::linux_atomic_abi::dg_linux_atomic_spawn(
                    &request,
                    &mut pidfd,
                    &mut mask.original,
                    &mut mask.active,
                )
            };
            if pid < 0 {
                let error = ChildError::io(
                    "atomic CLONE_PIDFD birth",
                    io::Error::from_raw_os_error(-pid as i32),
                );
                return Err(LinuxAtomicLaunchFailure::before_birth(
                    ChildSpawnError::Operation(error.with_cleanup(mask.restore())),
                ));
            }
            // 内核成功原子返还原fd；到adopt之间不分配、不回调、不格式化、无可失败转换。
            let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(pidfd) };
            let exit = LinuxAtomicExit::adopt(fd, pid as i32);
            let mut child = LinuxAtomicChild::adopt(exit, self.pipes);
            let observed = catch_unwind(AssertUnwindSafe(|| -> Result<(), ChildSpawnError<E>> {
                mask.restore()?;
                child.configure()?;
                if let Some(observer) = observer {
                    observer(&mut child)?;
                }
                LinuxAtomicHandshake::run(&mut child, deadline, checkpoint)
            }));
            match observed {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(LinuxAtomicLaunchFailure::after_birth(error, child)),
                Err(payload) => {
                    let cleanup = child.cleanup();
                    if let Err(error) = cleanup {
                        if !child.reaped() {
                            *unwind_owner = Some(child);
                        }
                        // 先交还原 owner，诊断失败不能替换原 panic payload。
                        let _ = writeln!(
                            std::io::stderr(),
                            "atomic launch panic cleanup failed: {error}"
                        );
                    }
                    // payload本身不替换；外部有限宿主槽能在catch_unwind之后显式回收原owner。
                    resume_unwind(payload);
                }
            }
            drop(self.image);
            Ok(child)
        }
    }
}

fn validate_material(
    image: &File,
    args: &[CString],
    environment: &[CString],
) -> Result<(), ChildError> {
    if args.is_empty() || args.len() > 256 || environment.len() > 128 {
        return Err(ChildError::Unsupported(
            "atomic argument count exceeds fixed preparation bound",
        ));
    }
    let mut total = 0_usize;
    for bytes in args
        .iter()
        .chain(environment)
        .map(|text| text.as_bytes_with_nul())
    {
        total = total
            .checked_add(bytes.len())
            .ok_or(ChildError::Unsupported("atomic argument bytes overflow"))?;
        if total > 1024 * 1024 {
            return Err(ChildError::Unsupported(
                "atomic argument bytes exceed fixed preparation bound",
            ));
        }
    }
    for variable in environment {
        let bytes = variable.as_bytes();
        let split = bytes.iter().position(|byte| *byte == b'=');
        let Some(split) = split.filter(|index| *index > 0) else {
            return Err(ChildError::Unsupported(
                "invalid explicit atomic environment",
            ));
        };
        let key = &bytes[..split];
        if key.starts_with(b"LD_") || matches!(key, b"GLIBC_TUNABLES" | b"GCONV_PATH") {
            return Err(ChildError::Unsupported(
                "loader overrides are forbidden in atomic environment",
            ));
        }
    }
    let metadata = image
        .metadata()
        .map_err(|error| ChildError::io("held ELF metadata", error))?;
    if !metadata.is_file() {
        return Err(ChildError::Unsupported(
            "held image is not a regular ELF file",
        ));
    }
    let mut header = [0_u8; 20];
    image
        .read_exact_at(&mut header, 0)
        .map_err(|error| ChildError::io("held ELF header", error))?;
    #[cfg(target_arch = "x86_64")]
    let machine = 62;
    #[cfg(target_arch = "aarch64")]
    let machine = 183;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let machine = 0;
    if header[..4] != *b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != machine
    {
        return Err(ChildError::Unsupported(
            "held image ELF architecture is unsupported",
        ));
    }
    Ok(())
}
