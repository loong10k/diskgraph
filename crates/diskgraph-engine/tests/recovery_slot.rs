use diskgraph_engine::recovery_slot::{SlotError, SlotReservation};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};
fn open(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn assert_unconfirmed_after_lock_release(path: &Path) {
    // Unix 的并行 fork 可能短暂继承原锁，直到 exec 关闭 CLOEXEC 文件描述符。
    // Busy 仍是安全拒绝；只在同一观察期限内等待锁释放，不允许成功认领或改写 RESERVED。
    let until = deadline();
    let rejection = loop {
        match SlotReservation::acquire(open(path), until) {
            Err(SlotError::Busy) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            result => break result.err(),
        }
    };
    assert!(
        matches!(rejection, Some(SlotError::Unconfirmed)),
        "unconfirmed record must refuse admission; actual rejection: {rejection:?}"
    );
}
fn child(path: &Path, mode: &str) -> std::process::ExitStatus {
    let process = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "slot_child", "--nocapture"])
        .env("DG_SLOT_TEST_PATH", path)
        .env("DG_SLOT_TEST_MODE", mode)
        .spawn()
        .unwrap();
    let mut guard = SlotTestChild {
        child: Some(process),
    };
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = guard.child.as_mut().unwrap().try_wait().unwrap() {
            guard.child.take();
            return status;
        }
        assert!(
            Instant::now() < until,
            "isolated child did not finish under original observation deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn real_other_process_is_blocked_then_prebirth_abort_allows_reuse() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    let file = File::create_new(&p).unwrap();
    let slot = SlotReservation::acquire(file, deadline()).unwrap();
    assert_eq!(child(&p, "claim").code(), Some(73));
    slot.abort_before_birth(deadline()).unwrap();
    assert!(child(&p, "claim").success());
}
#[test]
fn actual_process_death_releases_lock_but_not_active_record() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    drop(File::create_new(&p).unwrap());
    assert_eq!(child(&p, "die").code(), Some(42));
    let bytes = std::fs::read(&p).unwrap();
    assert_unconfirmed_after_lock_release(&p);
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
}
#[test]
fn dropping_reserved_owner_does_not_fake_completion() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    let slot = SlotReservation::acquire(File::create_new(&p).unwrap(), deadline()).unwrap();
    drop(slot);
    assert_unconfirmed_after_lock_release(&p);
}
#[test]
fn invalid_records_are_refused_without_modification() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    for bytes in [
        b"broken".as_slice(),
        b"DGSL99C\n".as_slice(),
        b"DGSL01A\nextra".as_slice(),
    ] {
        std::fs::write(&p, bytes).unwrap();
        assert_invalid_after_lock_release(&p, deadline());
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
    }
}

/// 参数：损坏槽与原观察期限；返回：精确 InvalidRecord 拒绝，不改写正文。
fn assert_invalid_after_lock_release(path: &Path, until: Instant) {
    // 锁优先于正文校验；Busy 是拒绝而非损坏正文的观察，不得据此修改记录。
    // 原固定期限内等实际锁释放；只允许最终 InvalidRecord，不接受成功或其他错误。
    let rejection = loop {
        match SlotReservation::acquire(open(path), until) {
            Err(SlotError::Busy) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1));
            }
            result => break result.err(),
        }
    };
    assert!(
        matches!(rejection, Some(SlotError::InvalidRecord)),
        "expected invalid record after lock release, got {rejection:?}"
    );
}

#[test]
fn actual_child_lock_delays_invalid_record_observation_without_repair() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("slot");
    let ready = directory.path().join("ready");
    let release = directory.path().join("release");
    let bytes = b"DGSL99C\n";
    std::fs::write(&path, bytes).unwrap();
    let process = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "slot_child", "--nocapture"])
        .env("DG_SLOT_TEST_PATH", &path)
        .env("DG_SLOT_TEST_MODE", "invalid-held")
        .env("DG_SLOT_TEST_READY", &ready)
        .env("DG_SLOT_TEST_RELEASE", &release)
        .spawn()
        .unwrap();
    let mut child = SlotTestChild {
        child: Some(process),
    };
    let until = deadline();
    while !ready.exists() {
        assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_none());
        assert!(Instant::now() < until, "child did not acquire actual lock");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(
        SlotReservation::acquire(open(&path), until),
        Err(SlotError::Busy)
    ));
    std::fs::write(release, b"release original lock").unwrap();
    assert_invalid_after_lock_release(&path, until);
    loop {
        if let Some(status) = child.child.as_mut().unwrap().try_wait().unwrap() {
            assert!(status.success());
            child.child.take();
            break;
        }
        assert!(Instant::now() < until, "actual lock holder has not exited");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
#[test]
fn original_expiry_refuses_before_writing_a_virgin_slot() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    assert!(matches!(
        SlotReservation::acquire(
            File::create_new(&p).unwrap(),
            Instant::now() - Duration::from_secs(1)
        ),
        Err(SlotError::Deadline)
    ));
    assert_eq!(std::fs::metadata(p).unwrap().len(), 0);
}
#[test]
#[ignore = "invoked only as actual isolated child by parent qualification"]
fn slot_child() {
    let path = std::env::var_os("DG_SLOT_TEST_PATH").expect("isolated child path");
    let mode = std::env::var("DG_SLOT_TEST_MODE").unwrap();
    if mode == "invalid-held" {
        let file = open(Path::new(&path));
        file.try_lock().unwrap();
        let ready = std::env::var_os("DG_SLOT_TEST_READY").unwrap();
        let release = std::env::var_os("DG_SLOT_TEST_RELEASE").unwrap();
        std::fs::write(ready, b"actual invalid record lock held").unwrap();
        let until = deadline();
        while !Path::new(&release).exists() {
            assert!(Instant::now() < until, "parent did not request release");
            std::thread::sleep(Duration::from_millis(2));
        }
        // 明确制造释放前的观察竞态；不改写原记录，返回后原 held file 才关闭。
        std::thread::sleep(Duration::from_millis(100));
        drop(file);
        return;
    }
    #[cfg(unix)]
    if mode == "inherited" {
        use std::os::fd::FromRawFd;
        // 该 fd 仅由父测试在子进程 pre_exec 中 dup2；不授予产品监督启动能力。
        let inherited = unsafe { File::from_raw_fd(198) };
        use std::os::unix::fs::MetadataExt;
        let actual = inherited.metadata().unwrap();
        let expected = std::fs::metadata(&path).unwrap();
        assert_eq!(
            (actual.dev(), actual.ino()),
            (expected.dev(), expected.ino())
        );
        let ready = std::env::var_os("DG_SLOT_TEST_READY").unwrap();
        std::fs::write(ready, b"original inherited slot held").unwrap();
        let until = deadline();
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(inherited);
        return;
    }
    #[cfg(windows)]
    if mode == "inherited-windows" {
        use std::io::Read;
        let mut bytes = [0_u8; 8];
        let error = std::io::stdin().read_exact(&mut bytes).unwrap_err();
        // LockFileEx 的锁属于原进程；继承文件句柄不能访问父进程锁定的区域。
        assert_eq!(error.raw_os_error(), Some(33));
        let ready = std::env::var_os("DG_SLOT_TEST_READY").unwrap();
        std::fs::write(ready, b"inherited handle cannot read parent lock").unwrap();
        let until = deadline();
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    match SlotReservation::acquire(open(Path::new(&path)), deadline()) {
        Err(SlotError::Busy) => std::process::exit(73),
        Err(error) => panic!("unexpected child slot rejection: {error}"),
        Ok(slot) => {
            if mode == "die" {
                let mut active = slot.activate(deadline()).unwrap();
                active.verify_active(deadline()).unwrap();
                std::process::exit(42);
            }
            slot.abort_before_birth(deadline()).unwrap();
        }
    }
}

/// 隔离测试原子进程的唯一兜底 owner；来源：监督槽资格夹具，无 Java 对等对象。
struct SlotTestChild {
    child: Option<std::process::Child>,
}
impl Drop for SlotTestChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn expiry_during_reserved_phase_preserves_unconfirmed_record() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    let slot = SlotReservation::acquire(File::create_new(&p).unwrap(), deadline()).unwrap();
    // Windows 的原生独占锁也可能禁止另一个句柄读取正文；不在持锁时旁路读取。
    let original = b"DGSL01R\n";
    assert!(matches!(
        slot.abort_before_birth(Instant::now() - Duration::from_secs(1)),
        Err(SlotError::Deadline)
    ));
    assert_eq!(std::fs::read(&p).unwrap(), original);
    assert_unconfirmed_after_lock_release(&p);
}

#[test]
fn append_mode_cannot_return_an_ambiguous_reserved_owner() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    std::fs::write(&p, b"DGSL01C\n").unwrap();
    let file = OpenOptions::new().read(true).append(true).open(&p).unwrap();
    assert!(matches!(
        SlotReservation::acquire(file, deadline()),
        Err(SlotError::InvalidRecord | SlotError::Io(_))
    ));
}

#[cfg(unix)]
#[test]
fn inherited_original_lock_survives_frontend_close_until_actual_child_death() {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("slot");
    let ready = directory.path().join("ready");
    let original = File::create_new(&path).unwrap();
    let original_fd = original.as_raw_fd();
    let active = SlotReservation::acquire(original, deadline())
        .unwrap()
        .activate(deadline())
        .unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", "slot_child", "--nocapture"])
        .env("DG_SLOT_TEST_PATH", &path)
        .env("DG_SLOT_TEST_MODE", "inherited")
        .env("DG_SLOT_TEST_READY", &ready);
    // 仅测试夹具使用 pre_exec；回调只调用 async-signal-safe dup2，不改变父进程 fd 标志。
    // 实际产品启动仍必须使用受信原生 launcher，而非本测试的 Command 路径。
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(original_fd, 198) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = SlotTestChild {
        child: Some(command.spawn().unwrap()),
    };
    let until = Instant::now() + Duration::from_secs(3);
    while !ready.exists() {
        assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_none());
        assert!(
            Instant::now() < until,
            "original inherited child never became ready"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // 实际关闭前端原 fd；第二次打开同一 inode 仍必须被原子进程的原锁拒绝。
    drop(active);
    assert!(matches!(
        SlotReservation::acquire(open(&path), deadline()),
        Err(SlotError::Busy)
    ));
    child.child.as_mut().unwrap().kill().unwrap();
    child.child.as_mut().unwrap().wait().unwrap();
    child.child.take();
    // 子进程死亡只释放内核锁；ACTIVE 不能被误写为 CLEAN 或自动允许新工作。
    assert_unconfirmed_after_lock_release(&path);
    assert_eq!(std::fs::read(path).unwrap(), b"DGSL01A\n");
}

#[cfg(windows)]
#[test]
fn inherited_windows_handle_does_not_transfer_original_process_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("slot");
    let ready = directory.path().join("ready");
    let original = File::create_new(&path).unwrap();
    // Stdio 只为隔离资格测试继承同一文件对象；不是产品监督启动入口。
    let inherited = original.try_clone().unwrap();
    let active = SlotReservation::acquire(original, deadline())
        .unwrap()
        .activate(deadline())
        .unwrap();
    let process = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "slot_child", "--nocapture"])
        .env("DG_SLOT_TEST_PATH", &path)
        .env("DG_SLOT_TEST_MODE", "inherited-windows")
        .env("DG_SLOT_TEST_READY", &ready)
        .stdin(std::process::Stdio::from(inherited))
        .spawn()
        .unwrap();
    let mut child = SlotTestChild {
        child: Some(process),
    };
    let until = Instant::now() + Duration::from_secs(3);
    while !ready.exists() {
        assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_none());
        assert!(
            Instant::now() < until,
            "inherited Windows lock rejection not observed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(active);
    // 等待原 OS 释放，不重新计时、不创建替代槽；活动记录必须仍拒绝新工作。
    loop {
        assert!(
            Instant::now() < until,
            "original parent lock did not release"
        );
        match SlotReservation::acquire(open(&path), until) {
            Err(SlotError::Busy) => std::thread::sleep(Duration::from_millis(10)),
            Err(SlotError::Unconfirmed) => break,
            Err(error) => panic!("unexpected inherited lock error: {error:?}"),
            Ok(_) => panic!("inherited lock unexpectedly admitted new work"),
        }
    }
    assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_none());
    child.child.as_mut().unwrap().kill().unwrap();
    child.child.as_mut().unwrap().wait().unwrap();
    child.child.take();
    assert_eq!(std::fs::read(path).unwrap(), b"DGSL01A\n");
}
