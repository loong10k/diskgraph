//! 原生目录删除失败和session代次的真实临时文件回归；不模拟OS成功。
use super::ProbeLimits;
use super::git_private_directory::GitPrivateDirectory;
use super::probe_budget::ProbeBudget;
use crate::ProbeHost;
use std::os::windows::fs::OpenOptionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

#[test]
fn real_directory_delete_failure_keeps_session_slot_until_native_release() {
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let mut budget = Some(ProbeBudget::new(&ProbeLimits::default()).unwrap());
    budget
        .as_mut()
        .unwrap()
        .bind_probe_host(Arc::clone(&host.registry))
        .unwrap();
    let mut private = GitPrivateDirectory::new(budget.as_mut().unwrap()).unwrap();
    let root = private.path().to_owned();
    let file = root.join("deny-delete");
    private
        .write(&file, b"held", budget.as_mut().unwrap())
        .unwrap();
    let mut held = Some(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(file)
            .unwrap(),
    );
    let observed = catch_unwind(AssertUnwindSafe(|| {
        let error = private.complete(Ok(7)).unwrap_err();
        assert!(
            error.contains("cleanup"),
            "native deletion failure absent: {error}"
        );
        assert!(root.exists());
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        assert!(
            !recovery.drain().unwrap(),
            "live session must not be reclaimed"
        );
        drop(budget.take());
        assert!(
            recovery.drain().is_err(),
            "actual denied native deletion must retain the owner"
        );
        assert!(host.registry.reserve().is_err());
        drop(held.take());
        assert!(recovery.drain().unwrap());
        assert!(!root.exists());
        let next = host.registry.reserve().unwrap();
        assert!(
            private.complete(Ok(())).is_err(),
            "old retained facade must not pretend it recovered"
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        drop(next);
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
    }));
    // finally只解除本测试的真实删除阻止句柄，再让同Recovery重试；不直接删除路径冒充成功。
    drop(held.take());
    drop(budget.take());
    drop(private);
    let cleaned = recovery.drain();
    if !matches!(cleaned, Ok(true)) {
        std::mem::forget(host);
        std::mem::forget(recovery);
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!("native directory recovery incomplete; responsibility retained: {cleaned:?}");
    }
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn borrowed_directory_prevents_session_reuse_after_budget_drop() {
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    budget.bind_probe_host(Arc::clone(&host.registry)).unwrap();
    let mut private = GitPrivateDirectory::new(&mut budget).unwrap();
    let root = private.path().to_owned();
    drop(budget);
    let observed = catch_unwind(AssertUnwindSafe(|| {
        assert!(host.registry.reserve().is_err());
        assert!(!recovery.drain().unwrap());
        assert!(
            root.is_dir(),
            "borrowed original directory must remain alive"
        );
        private.complete(Ok(())).unwrap();
        assert!(recovery.drain().unwrap());
        let next = host.registry.reserve().unwrap();
        private.complete(Ok(())).unwrap();
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        drop(next);
    }));
    drop(private);
    let cleaned = recovery.drain();
    if !matches!(cleaned, Ok(true)) {
        std::mem::forget(host);
        std::mem::forget(recovery);
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!("borrowed directory recovery incomplete: {cleaned:?}");
    }
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn explicit_directory_retry_reborrows_only_original_live_session_owner() {
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let mut budget = Some(ProbeBudget::new(&ProbeLimits::default()).unwrap());
    budget
        .as_mut()
        .unwrap()
        .bind_probe_host(Arc::clone(&host.registry))
        .unwrap();
    let mut private = GitPrivateDirectory::new(budget.as_mut().unwrap()).unwrap();
    let root = private.path().to_owned();
    let path = root.join("deny-delete");
    private
        .write(&path, b"original payload", budget.as_mut().unwrap())
        .unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .unwrap();
    let denied = private.complete(Ok(7)).unwrap_err();
    assert!(
        denied.contains("cleanup"),
        "original denied deletion: {denied}"
    );
    assert_eq!(recovery.occupied_slots().unwrap(), 1);
    assert!(
        !recovery.drain().unwrap(),
        "live original session cannot be reclaimed"
    );
    assert!(
        host.registry.reserve().is_err(),
        "failed deletion retains the original capacity"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original payload");
    drop(held);
    // 原代次原 owner 重新借出并实际删除；不得按名称建立新 owner。
    eprintln!("DG_WINDOWS_ORIGINAL_DIRECTORY_RETRY_RED_READY=1");
    let observed = catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(private.complete(Ok(8)).unwrap(), 8);
    }));
    if let Err(payload) = observed {
        // 旧实现的确切 RED 也实际恢复原 owner；继续原 payload，不遗留测试私有数据。
        drop(budget.take());
        assert!(recovery.drain().unwrap());
        assert!(!root.exists());
        eprintln!("DG_WINDOWS_ORIGINAL_DIRECTORY_RETRY_RED_CLEANUP=1");
        resume_unwind(payload);
    }
    assert!(!root.exists());
    assert_eq!(
        recovery.occupied_slots().unwrap(),
        1,
        "live session still holds its original slot"
    );
    drop(budget.take());
    assert!(recovery.drain().unwrap());
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    eprintln!("DG_WINDOWS_ORIGINAL_DIRECTORY_EXPLICIT_RETRY=1");
}

#[test]
fn directory_retry_after_session_release_cannot_take_recovery_owner() {
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let mut budget = Some(ProbeBudget::new(&ProbeLimits::default()).unwrap());
    budget
        .as_mut()
        .unwrap()
        .bind_probe_host(Arc::clone(&host.registry))
        .unwrap();
    let mut private = GitPrivateDirectory::new(budget.as_mut().unwrap()).unwrap();
    let root = private.path().to_owned();
    let path = root.join("deny-delete");
    private
        .write(&path, b"original payload", budget.as_mut().unwrap())
        .unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .unwrap();
    let denied = private.complete(Ok(7)).unwrap_err();
    assert!(
        denied.contains("cleanup"),
        "original denied deletion: {denied}"
    );
    drop(budget.take());
    assert!(
        private
            .complete(Ok(8))
            .unwrap_err()
            .contains("retained for recovery")
    );
    assert_eq!(recovery.occupied_slots().unwrap(), 1);
    drop(private);
    assert_eq!(
        recovery.occupied_slots().unwrap(),
        1,
        "Drop must not reborrow the owner"
    );
    assert!(
        recovery.drain().is_err(),
        "original denied deletion still fails"
    );
    assert!(root.exists());
    drop(held);
    assert!(recovery.drain().unwrap());
    assert!(!root.exists());
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    eprintln!("DG_WINDOWS_RELEASED_SESSION_CANNOT_REBORROW_DIRECTORY=1");
}
