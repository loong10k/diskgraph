//! 修改原 held fd 的真实追加模式，验证退休写入异常不会释放原锁或修复损坏状态。
use super::{SlotError, SlotReservation};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
#[test]
fn original_fd_write_anomaly_keeps_lock_and_refuses_retry_without_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slot");
    let mut active = SlotReservation::acquire(File::create_new(&path).unwrap(), deadline())
        .unwrap()
        .activate(deadline())
        .unwrap();
    let fd = active.file.as_raw_fd();
    // 直接改变原同一 held fd 的 OS 写入模式，不更换文件、目录或恢复 owner。
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_APPEND) },
        0
    );
    assert!(matches!(
        active.confirm_original_cleanup(deadline()),
        Err(SlotError::InvalidRecord)
    ));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, b"DGSL01A\nDGSL01C\n");
    assert!(matches!(
        SlotReservation::acquire(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap(),
            deadline()
        ),
        Err(SlotError::Busy)
    ));
    assert!(matches!(
        active.confirm_original_cleanup(deadline()),
        Err(SlotError::InvalidRecord)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    drop(active);
    assert!(matches!(
        SlotReservation::acquire(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap(),
            deadline()
        ),
        Err(SlotError::InvalidRecord)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn confirmed_cleanup_releases_lock_while_inherited_child_remains_alive() {
    const MARKER: &str = "DISKGRAPH_SLOT_INHERITANCE_ISOLATED";
    if std::env::var_os(MARKER).is_none() {
        // 独立测试进程只运行本夹具，避免 fork 继承并暂留其它并发测试的真实锁。
        struct FixtureOwner(std::process::Child);
        impl Drop for FixtureOwner {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let stdout = tempfile::tempfile().unwrap();
        let stderr = tempfile::tempfile().unwrap();
        let mut owner = FixtureOwner(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "recovery_slot::active_slot_tests::confirmed_cleanup_releases_lock_while_inherited_child_remains_alive", "--nocapture"])
            .env(MARKER, "1")
            .stdout(stdout.try_clone().unwrap()).stderr(stderr.try_clone().unwrap())
            .spawn().unwrap());
        let until = deadline();
        let status = loop {
            if let Some(status) = owner.0.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < until, "isolated fixture exceeded deadline");
            std::thread::sleep(Duration::from_millis(1));
        };
        use std::io::{Read, Seek, SeekFrom};
        let mut diagnostics = String::new();
        for mut file in [stdout, stderr] {
            file.seek(SeekFrom::Start(0)).unwrap();
            file.take(16384).read_to_string(&mut diagnostics).unwrap();
        }
        assert!(status.success(), "isolated fixture failed: {diagnostics}");
        return;
    }
    // 子进程在 fork 后仅调用 async-signal-safe 的 C 操作；清理 guard 保证 RED 也能退出。
    struct InheritedChild {
        pid: libc::pid_t,
        release: libc::c_int,
    }
    impl Drop for InheritedChild {
        fn drop(&mut self) {
            if self.pid <= 0 {
                return;
            }
            let byte = [1_u8];
            unsafe {
                libc::write(self.release, byte.as_ptr().cast(), 1);
                libc::close(self.release);
                let mut status = 0;
                let until = Instant::now() + Duration::from_secs(2);
                loop {
                    let result = libc::waitpid(self.pid, &mut status, libc::WNOHANG);
                    if result == self.pid {
                        break;
                    }
                    if result < 0
                        && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                    {
                        break;
                    }
                    if Instant::now() >= until {
                        libc::kill(self.pid, libc::SIGKILL);
                        while libc::waitpid(self.pid, &mut status, 0) < 0 {
                            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                                break;
                            }
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slot");
    let mut active = SlotReservation::acquire(File::create_new(&path).unwrap(), deadline())
        .unwrap()
        .activate(deadline())
        .unwrap();
    let mut pipe = [-1; 2];
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            libc::close(pipe[1]);
            let mut byte = 0_u8;
            let result = libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
            libc::_exit(if result >= 0 { 0 } else { 2 });
        }
    }
    unsafe {
        libc::close(pipe[0]);
    }
    let mut child = InheritedChild {
        pid,
        release: pipe[1],
    };
    let open = || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap()
    };
    assert!(matches!(
        SlotReservation::acquire(open(), deadline()),
        Err(SlotError::Busy)
    ));
    active.confirm_original_cleanup(deadline()).unwrap();
    drop(active);
    // 子进程仍等待原 release 管道，原退休容量必须能被新 owner 认领。
    let next = SlotReservation::acquire(open(), deadline())
        .unwrap()
        .activate(deadline())
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"DGSL01A\n");
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(child.pid, &mut status, libc::WNOHANG) },
        0,
        "inherited child must still be alive after new owner acquires"
    );
    let byte = [1_u8];
    assert_eq!(
        unsafe { libc::write(child.release, byte.as_ptr().cast(), 1) },
        1
    );
    let until = deadline();
    loop {
        let observed = unsafe { libc::waitpid(child.pid, &mut status, libc::WNOHANG) };
        if observed == child.pid {
            child.pid = -1;
            break;
        }
        assert!(
            observed == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
        );
        assert!(
            Instant::now() < until,
            "inherited child did not exit after release"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    unsafe {
        libc::close(child.release);
    }
    assert!(libc::WIFEXITED(status));
    assert_eq!(libc::WEXITSTATUS(status), 0);
    drop(next);
    drop(child);
}
