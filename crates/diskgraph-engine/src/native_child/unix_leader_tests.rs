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

#[cfg(target_os = "macos")]
#[test]
fn nonpositive_native_pid_cannot_reap_an_unrelated_child() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_child::unix_leader_tests::nonpositive_native_pid_isolated_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated wait ownership fixture failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
        "fixture did not actually run"
    );
    println!("DG_NONPOSITIVE_WAIT_FIXTURE_VERIFIED pid=0,-1,-42");
}

/// 隔离进程中创建真实独立 Child，避免故障实现的 waitpid(0/-1) 干扰并行测试。
#[cfg(target_os = "macos")]
#[test]
#[ignore = "invoked by the real isolated ownership regression"]
fn nonpositive_native_pid_isolated_fixture() {
    for pid in [0, -1, -42] {
        let mut unrelated = Command::new("/bin/sleep").arg("0.1").spawn().unwrap();
        // 故意模拟未出生槽及损坏身份；此处没有宣称拥有该非正 PID。
        let mut leader = if pid == 0 {
            UnixLeader::prepare_native()
        } else {
            UnixLeader::Native { pid, status: None }
        };
        let first = leader.wait();
        // 在任何断言之前由真实 Child owner 收场。旧实现可能已错误地回收它。
        let original_wait = unrelated.wait();
        eprintln!(
            "DG_NONPOSITIVE_WAIT pid={pid} original_wait_errno={:?} invalid_input={}",
            original_wait
                .as_ref()
                .err()
                .and_then(std::io::Error::raw_os_error),
            matches!(&first, Err(error) if error.kind() == std::io::ErrorKind::InvalidInput)
        );
        assert!(
            matches!(first, Err(ref error) if error.kind() == std::io::ErrorKind::InvalidInput),
            "nonpositive leader entered native wait: {first:?}"
        );
        assert!(
            original_wait.unwrap().success(),
            "unrelated child's own wait was consumed"
        );
        assert_eq!(
            leader.wait().unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_and_standard_poll_wait_keep_live_owner_and_cache_only_actual_reaping() {
    use std::time::{Duration, Instant};
    for native in [false, true] {
        let mut leader = if native {
            let mut pid = 0;
            let arguments = [c"sleep".as_ptr(), c"2".as_ptr(), std::ptr::null()];
            let environment = [std::ptr::null::<libc::c_char>()];
            assert_eq!(
                unsafe {
                    libc::posix_spawn(
                        &mut pid,
                        c"/bin/sleep".as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        arguments.as_ptr().cast(),
                        environment.as_ptr().cast(),
                    )
                },
                0
            );
            unsafe { UnixLeader::from_native(pid) }
        } else {
            UnixLeader::from_standard(Command::new("/bin/sleep").arg("2").spawn().unwrap())
        };
        let pid = leader.id();
        let started = Instant::now();
        let live = leader.poll_wait().unwrap();
        let elapsed = started.elapsed();
        // 在断言前收场；若回归错误地已消费 wait，则不再向旧数字 PID 发信号。
        if live.is_none() {
            assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let actual = loop {
            if let Some(status) = leader.poll_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(live.is_none());
        assert!(elapsed < Duration::from_millis(500));
        assert_eq!(leader.poll_wait().unwrap(), Some(actual));
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
}
