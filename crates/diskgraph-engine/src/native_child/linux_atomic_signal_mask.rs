use super::ChildError;
use std::io::{self, Write};

/// 在原子 owner 接管前保持当前调用线程的 mask，随后恢复宿主原值。
/// 来源：Linux native64 rt_sigprocmask，绝不修改宿主全进程 handler。
pub(super) struct LinuxAtomicSignalMask {
    pub(super) original: u64,
    pub(super) active: i32,
}

impl LinuxAtomicSignalMask {
    /// 参数：无；返回：出生前就存在的未激活恢复守卫。
    pub(super) fn new() -> Self {
        Self {
            original: 0,
            active: 0,
        }
    }

    /// 参数：无；返回：恢复真实原 mask 的结果；失败不假称已经恢复。
    pub(super) fn restore(&mut self) -> Result<(), ChildError> {
        if self.active == 0 {
            return Ok(());
        }
        #[cfg(all(
            any(target_arch = "x86_64", target_arch = "aarch64"),
            target_endian = "little",
            target_pointer_width = "64"
        ))]
        {
            let result =
                unsafe { super::linux_atomic_abi::dg_linux_atomic_restore_mask(self.original) };
            if result < 0 {
                return Err(ChildError::io(
                    "restore atomic launcher signal mask",
                    io::Error::from_raw_os_error(-result as i32),
                ));
            }
            self.active = 0;
            Ok(())
        }
        #[cfg(not(all(
            any(target_arch = "x86_64", target_arch = "aarch64"),
            target_endian = "little",
            target_pointer_width = "64"
        )))]
        Err(ChildError::Unsupported(
            "atomic launcher signal ABI is unsupported",
        ))
    }
}

impl Drop for LinuxAtomicSignalMask {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            let _ = writeln!(
                std::io::stderr(),
                "atomic launcher mask cleanup failed: {error}"
            );
        }
    }
}
