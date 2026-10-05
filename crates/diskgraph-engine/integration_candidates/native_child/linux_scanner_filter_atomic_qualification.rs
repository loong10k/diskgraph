use super::ChildError;

/// 固定 helper 的用户线程派生约束，不限制文件访问或授予任务权限。
/// 来源：Linux seccomp_data/audit ABI 与 legacy clone 的线程组条件。
pub(super) struct LinuxScannerFilter {
    instructions: [libc::sock_filter; 64],
    count: usize,
}

impl LinuxScannerFilter {
    /// 借用父端已构建 BPF，供纯 syscall child 使用，不重建或放宽过滤器。
    /// 参数：无；返回：只在本对象存活期间有效的原 sock_fprog。
    pub(super) fn program(&self) -> libc::sock_fprog {
        libc::sock_fprog {
            len: self.count as u16,
            filter: self.instructions.as_ptr().cast_mut(),
        }
    }

    /// 在父进程预构建有界过滤器。参数：无；返回：受支持 ABI 的固定 BPF 或能力错误。
    pub(super) fn new() -> Result<Self, ChildError> {
        #[cfg(not(all(
            any(target_arch = "x86_64", target_arch = "aarch64"),
            target_endian = "little"
        )))]
        return Err(ChildError::Unsupported(
            "scanner seccomp syscall ABI is unsupported",
        ));
        #[cfg(all(
            any(target_arch = "x86_64", target_arch = "aarch64"),
            target_endian = "little"
        ))]
        {
            let mut filter = Self {
                instructions: [libc::sock_filter {
                    code: 0,
                    jt: 0,
                    jf: 0,
                    k: 0,
                }; 64],
                count: 0,
            };
            #[cfg(target_arch = "x86_64")]
            let arch = 0xc000_003e;
            #[cfg(target_arch = "aarch64")]
            let arch = 0xc000_00b7;
            filter.load(4); // seccomp_data.arch，先拒其它 syscall ABI。
            filter.jump(libc::BPF_JEQ, arch, 1, 0);
            filter.ret(0x8000_0000); // SECCOMP_RET_KILL_PROCESS
            filter.load(0); // seccomp_data.nr
            #[cfg(target_arch = "x86_64")]
            {
                // x32 使用相同 audit.arch，必须独立拒高位；历史无高位 x32 区间也拒绝。
                filter.jump(libc::BPF_JSET, 0x4000_0000, 0, 1);
                filter.ret(0x8000_0000);
                filter.jump(libc::BPF_JGE, 512, 0, 2);
                filter.jump(libc::BPF_JGT, 547, 1, 0);
                filter.ret(0x8000_0000);
            }
            filter.jump(libc::BPF_JEQ, libc::SYS_clone3 as u32, 0, 1);
            filter.ret(0x0005_0000 | libc::ENOSYS as u32);
            #[cfg(target_arch = "x86_64")]
            for number in [libc::SYS_fork, libc::SYS_vfork] {
                filter.deny(number as u32);
            }
            for number in [
                libc::SYS_setsid,
                libc::SYS_setpgid,
                libc::SYS_unshare,
                libc::SYS_setns,
                libc::SYS_ptrace,
                libc::SYS_io_uring_setup,
                libc::SYS_io_uring_enter,
                libc::SYS_io_uring_register,
            ] {
                filter.deny(number as u32);
            }
            filter.jump(libc::BPF_JEQ, libc::SYS_clone as u32, 1, 0);
            filter.ret(0x7fff_0000); // 其它现有 syscall 保持原扫描行为。
            // clone args[0] 是完整 u64；仅批准已知线程 flags，不能漏掉高32位。
            filter.load(20);
            filter.jump(libc::BPF_JEQ, 0, 1, 0);
            filter.ret(0x0005_0000 | libc::EPERM as u32);
            filter.load(16);
            let required = (libc::CLONE_VM | libc::CLONE_SIGHAND | libc::CLONE_THREAD) as u32;
            let allowed = required
                | (libc::CLONE_FS
                    | libc::CLONE_FILES
                    | libc::CLONE_SYSVSEM
                    | libc::CLONE_SETTLS
                    | libc::CLONE_PARENT_SETTID
                    | libc::CLONE_CHILD_SETTID
                    | libc::CLONE_CHILD_CLEARTID) as u32;
            filter.jump(libc::BPF_JSET, !allowed, 0, 1);
            filter.ret(0x0005_0000 | libc::EPERM as u32);
            filter.push(
                (libc::BPF_ALU | libc::BPF_AND | libc::BPF_K) as u16,
                0,
                0,
                required,
            );
            filter.jump(libc::BPF_JEQ, required, 1, 0);
            filter.ret(0x0005_0000 | libc::EPERM as u32);
            filter.ret(0x7fff_0000);
            Ok(filter)
        }
    }

    fn load(&mut self, offset: u32) {
        self.push(
            (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            0,
            0,
            offset,
        );
    }

    fn jump(&mut self, operation: u32, value: u32, yes: u8, no: u8) {
        self.push(
            (libc::BPF_JMP | operation | libc::BPF_K) as u16,
            yes,
            no,
            value,
        );
    }

    fn ret(&mut self, value: u32) {
        self.push((libc::BPF_RET | libc::BPF_K) as u16, 0, 0, value);
    }

    fn deny(&mut self, syscall: u32) {
        self.jump(libc::BPF_JEQ, syscall, 0, 1);
        self.ret(0x0005_0000 | libc::EPERM as u32);
    }

    fn push(&mut self, code: u16, jt: u8, jf: u8, k: u32) {
        self.instructions[self.count] = libc::sock_filter { code, jt, jf, k };
        self.count += 1;
    }
}
