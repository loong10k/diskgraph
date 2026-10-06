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
