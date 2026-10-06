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
    assert!(matches!(
        SlotReservation::acquire(open(&p), deadline()),
        Err(SlotError::Unconfirmed)
    ));
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
}
#[test]
fn dropping_reserved_owner_does_not_fake_completion() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("slot");
    let slot = SlotReservation::acquire(File::create_new(&p).unwrap(), deadline()).unwrap();
    drop(slot);
    assert!(matches!(
        SlotReservation::acquire(open(&p), deadline()),
        Err(SlotError::Unconfirmed)
    ));
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
        assert!(matches!(
            SlotReservation::acquire(open(&p), deadline()),
            Err(SlotError::InvalidRecord)
        ));
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
    }
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
    assert!(matches!(
        SlotReservation::acquire(open(&p), deadline()),
        Err(SlotError::Unconfirmed)
    ));
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
