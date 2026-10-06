//! 原 leader 所有权与重复 wait 合同；来源：原生 Rust Unix 独占进程生命周期。
use super::unix_leader::UnixLeader;
use std::process::Command;

#[test]
fn standard_child_wait_is_cached_after_actual_reaping() {
    let child = Command::new("/usr/bin/true").spawn().unwrap();
    let pid = child.id();
    let mut leader = UnixLeader::from_standard(child);
    assert_eq!(leader.id(), pid);
    assert!(leader.wait().unwrap().success());
    assert!(leader.wait().unwrap().success());
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn native_child_wait_consumes_only_original_pid_and_keeps_cached_exit_status() {
    for (path, args, expected) in [
        (
            c"/usr/bin/true",
            vec![c"true".as_ptr(), std::ptr::null()],
            0,
        ),
        (
            c"/bin/sh",
            vec![
                c"sh".as_ptr(),
                c"-c".as_ptr(),
                c"exit 7".as_ptr(),
                std::ptr::null(),
            ],
            7,
        ),
    ] {
        let mut pid = 0;
        let env = [std::ptr::null::<libc::c_char>()];
        // 测试仅启动固定系统程序，无生产安装或不可逃逸声明；成功后立即转移原 PID。
        let result = unsafe {
            libc::posix_spawn(
                &mut pid,
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                args.as_ptr().cast(),
                env.as_ptr().cast(),
            )
        };
        assert_eq!(result, 0);
        let mut leader = unsafe { UnixLeader::from_native(pid) };
        assert_eq!(leader.id(), pid as u32);
        assert_eq!(leader.wait().unwrap().code(), Some(expected));
        assert_eq!(leader.wait().unwrap().code(), Some(expected));
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
}
