//! 原生 seccomp 只对 EXEC 参数返回 EINVAL；不是旧内核模拟或生产策略实现。

use super::linux_memfd_policy_support::{isolated, original_deadline};
use super::linux_scan_image_execution_fixture::{MFD_EXEC, ScanImageExecutionFixture, raw_memfd};
use crate::EngineError;
use std::io;

#[test]
fn native_einval_for_exec_is_preserved_without_flag_removal_retry() {
    isolated(
        "native_child::linux_memfd_denial_tests::native_einval_for_exec_is_preserved_without_flag_removal_retry",
        || {
            let end = original_deadline();
            let fixture = ScanImageExecutionFixture::new(end);
            install_exec_denial();
            // 两次真实 syscall 证明仅 EXEC 被拒：去旗本会成功，但生产不允许这样回退。
            let error = raw_memfd(MFD_EXEC).expect_err("kernel denial must be observable");
            assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
            drop(raw_memfd(0).expect("native no-EXEC control must actually succeed"));
            match fixture.prepared() {
                Err(EngineError::Io(error)) => assert_eq!(error.raw_os_error(), Some(libc::EINVAL)),
                Err(other) => panic!("original memfd EINVAL was replaced: {other:?}"),
                Ok(_) => panic!("production accepted an image without mandatory MFD_EXEC"),
            }
            eprintln!(
                "MEMFD_EINVAL observed=true origin=isolated_seccomp old_kernel_qualification=false no_fallback=true"
            );
        },
    );
}

fn install_exec_denial() {
    if !cfg!(all(target_pointer_width = "64", target_endian = "little")) {
        panic!("missing_qualification: native64 little-endian BPF fixture required");
    }
    // syscall nr 在 offset0，第二参数低32位在 offset24；仅当前测试进程线程安装。
    let mut filters = [
        instruction(0x20, 0, 0, 0),
        instruction(0x15, 0, 3, libc::SYS_memfd_create as u32),
        instruction(0x20, 0, 0, 24),
        instruction(0x45, 0, 1, MFD_EXEC),
        instruction(0x06, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EINVAL as u32),
        instruction(0x06, 0, 0, libc::SECCOMP_RET_ALLOW),
    ];
    let program = libc::sock_fprog {
        len: filters.len() as u16,
        filter: filters.as_mut_ptr(),
    };
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
        0,
        "no_new_privs {:?}",
        io::Error::last_os_error()
    );
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) },
        0,
        "seccomp qualification {:?}",
        io::Error::last_os_error()
    );
}

fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}
