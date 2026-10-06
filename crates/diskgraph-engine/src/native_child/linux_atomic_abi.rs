use std::ffi::c_char;

/// 父端预构建的纯 syscall child 材料，不拥有权限、数据库或可展开回调。
/// 来源：原生 Rust/C 的固定 native64 CLONE_PIDFD 与 execveat ABI。
#[repr(C)]
pub(super) struct LinuxAtomicAbi {
    pub(super) fds: [i32; 5],
    pub(super) argv: *const *const c_char,
    pub(super) environment: *const *const c_char,
    pub(super) filter_count: u16,
    pub(super) filter: *const libc::sock_filter,
}

#[cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_endian = "little",
    target_pointer_width = "64"
))]
unsafe extern "C" {
    /// 参数：已准备 COW 材料、内核 pidfd 输出及原信号 mask；返回：原 PID 或负原 errno。
    pub(super) fn dg_linux_atomic_spawn(
        request: *const LinuxAtomicAbi,
        pidfd: *mut i32,
        old_mask: *mut u64,
        mask_active: *mut i32,
    ) -> libc::c_long;
    /// 参数：当前宿主线程原 mask；返回：纯 rt_sigprocmask 成功值或负原 errno。
    pub(super) fn dg_linux_atomic_restore_mask(mask: u64) -> libc::c_long;
}
