//! 原owner实际wait后槽复用的ABA回归；来源：PF-06有限恢复容量。
//! 不构造假child，不把cleanup当正常结果许可，旧预留无权释放后来请求。

use crate::scan_worker_registry::ScanWorkerRegistry;

#[test]
fn old_reservation_drop_cannot_clear_a_reused_slot_after_actual_child_reap() {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let first = registry.reserve().unwrap();
    #[cfg(target_os = "macos")]
    let (child, pid, _directory) = {
        use crate::native_child::UnixChild;
        use std::ffi::OsStr;
        use std::time::{Duration, Instant};
        let end = Instant::now() + Duration::from_secs(20);
        let directory = tempfile::tempdir().unwrap();
        let mut child = UnixChild::spawn_worker(
            &std::env::current_exe().unwrap(),
            &[
                OsStr::new("--exact"),
                OsStr::new("native_child::unix_normal_exit_fixture::normal_fixture"),
                OsStr::new("--nocapture"),
            ],
            &[
                (OsStr::new("DG_NORMAL_FIXTURE"), OsStr::new("live_end")),
                (
                    OsStr::new("DG_NORMAL_DIRECTORY"),
                    directory.path().as_os_str(),
                ),
            ],
            || {
                if Instant::now() >= end {
                    Err("original fixture deadline elapsed")
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        while !directory.path().join("heartbeat").exists() {
            assert!(
                Instant::now() < end,
                "actual live-child qualification failed"
            );
            std::thread::yield_now();
        }
        let scalar = |name: &str| {
            std::fs::read_to_string(directory.path().join(name))
                .unwrap()
                .parse::<i32>()
                .unwrap()
        };
        let pid = scalar("pid");
        assert_eq!(scalar("pgid"), pid);
        assert_eq!(scalar("sid"), pid);
        assert_eq!(scalar("ppid"), std::process::id() as i32);
        assert!(
            !child.poll().unwrap(),
            "real leader remains alive before retention"
        );
        (child, pid, directory)
    };
    #[cfg(target_os = "linux")]
    let (child, _fixture) = {
        let fixture = crate::scan_worker_recovery_fixture::ScanWorkerRecoveryFixture::new();
        let child = fixture.launch_original();
        assert!(!child.physically_exited().unwrap());
        assert!(!child.reaped());
        (child, fixture)
    };
    first.retain(child);
    assert_eq!(registry.occupied().unwrap(), 1);
    assert!(
        registry.drain().unwrap(),
        "original OS owner was actually cleaned/waited"
    );
    #[cfg(target_os = "macos")]
    {
        // 同一个原测试leader已被owner消费；此只核ECHILD，不以数字PID执行signal/reopen。
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
    assert_eq!(registry.occupied().unwrap(), 0);
    let second = registry.reserve().unwrap();
    assert_eq!(registry.occupied().unwrap(), 1);
    drop(first);
    let occupied_with_second_alive = registry.occupied();
    drop(second);
    assert_eq!(
        occupied_with_second_alive.unwrap(),
        1,
        "old reservation released a different request's slot"
    );
    assert_eq!(registry.occupied().unwrap(), 0);
}
