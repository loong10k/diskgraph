use std::ffi::c_char;

/// 出生前固定七描述符与专用身份材料；来源：linux_supervisor_spawn.h native64 ABI。
#[repr(C)]
pub(super) struct LinuxSupervisorAbi {
    pub(super) fds: [i32; 7],
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) argv: *const *const c_char,
    pub(super) environment: *const *const c_char,
    pub(super) group_count: u32,
    pub(super) groups: *const u32,
}

unsafe extern "C" {
    /// 参数：固定出生材料、原 pidfd 和 mask 输出；返回：原 PID 或负原 errno。
    pub(super) fn dg_linux_supervisor_spawn(
        request: *const LinuxSupervisorAbi,
        pidfd: *mut i32,
        old_mask: *mut u64,
        mask_active: *mut i32,
    ) -> libc::c_long;
}
