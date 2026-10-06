//! 原生共享冲突必须沿原期限恢复，不放宽 shareREAD/no-reparse 语义。
use super::ProbeLimits;
use super::git_directory_lease::GitDirectoryLease;
use super::probe_budget::ProbeBudget;
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::Ordering;
use std::time::Duration;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ADD_FILE, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};

fn blocker(path: &std::path::Path) -> std::fs::File {
    std::fs::OpenOptions::new()
        .access_mode(FILE_ADD_FILE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

#[test]
fn actual_shared_write_handle_releases_before_original_lease_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap();
    let held = blocker(&path);
    // 初始句柄确实参与共享冲突，属性句柄不能代替这个原生反控。
    let conflict = std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(&path)
        .unwrap_err();
    assert_eq!(
        conflict.raw_os_error(),
        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
    );
    println!("DG_WINDOWS_DIRECTORY_SHARING_RED_READY=1");
    let actor = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        drop(held);
    });
    let mut budget = ProbeBudget::new(&ProbeLimits {
        timeout: Duration::from_secs(2),
        ..ProbeLimits::default()
    })
    .unwrap();
    let result = GitDirectoryLease::open(&path, &mut budget);
    actor.join().unwrap();
    assert!(
        result.is_ok(),
        "actual shared directory lease must recover after original blocker release: {}",
        result.err().unwrap_or_default()
    );
    println!("DG_WINDOWS_DIRECTORY_SHARING_RETRY=1");
}

#[test]
fn original_expiry_and_cancellation_stop_held_sharing_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap();
    let _held = blocker(&path);
    let limits = ProbeLimits {
        timeout: Duration::from_millis(30),
        ..ProbeLimits::default()
    };
    let mut budget = ProbeBudget::new(&limits).unwrap();
    assert!(GitDirectoryLease::open(&path, &mut budget).is_err());
    assert!(matches!(
        budget.failure(),
        Some(super::probe_failure::ProbeFailure::Deadline)
    ));
    let limits = ProbeLimits::default();
    let cancel = std::sync::Arc::clone(&limits.cancel);
    let actor = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::Release);
    });
    let mut budget = ProbeBudget::new(&limits).unwrap();
    assert!(GitDirectoryLease::open(&path, &mut budget).is_err());
    actor.join().unwrap();
    assert!(matches!(
        budget.failure(),
        Some(super::probe_failure::ProbeFailure::Cancelled)
    ));
}
