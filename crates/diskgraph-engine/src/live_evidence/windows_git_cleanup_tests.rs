//! 实际生产Git目录创建、账本及句柄清理，不替代Windows原生执行证据。
use super::ProbeLimits;
use super::git_private_directory::GitPrivateDirectory;
use super::native_probe_test_budget::NativeProbeTestBudget;
use std::time::{Duration, Instant};

#[test]
fn product_cleanup_owner_does_not_pin_unrelated_sibling_names() {
    let mut probe = NativeProbeTestBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut probe).unwrap();
    let root = directory.path().to_owned();
    let sibling = tempfile::tempdir_in(root.parent().unwrap()).unwrap();
    let original = sibling.path().to_owned();
    let moved = original.with_extension("moved");
    std::fs::write(original.join("sentinel"), b"unrelated").unwrap();
    // 产品owner存活时无关同父对象仍可正常改名，原创建租约不能留在恢复状态中。
    rename_after_concurrent_creation(&original, &moved);
    rename_after_concurrent_creation(&moved, &original);
    assert_eq!(
        std::fs::read(original.join("sentinel")).unwrap(),
        b"unrelated"
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while let Err(error) = directory.complete::<()>(Ok(())) {
        assert!(
            Instant::now() < deadline,
            "cleanup remains incomplete: {error}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read(original.join("sentinel")).unwrap(),
        b"unrelated"
    );
}

#[test]
fn product_cleanup_removes_registered_tree_at_original_name() {
    let mut probe = NativeProbeTestBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut probe).unwrap();
    let root = directory.path().to_owned();
    let nested = root.join("nested");
    directory.create_dir_all(&nested, &mut probe).unwrap();
    directory
        .write(&nested.join("owned"), b"registered payload", &mut probe)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    // 正常路径必须实际删除登记树；拒绝改名不能代替正常完成的正控。
    loop {
        match directory.complete::<()>(Ok(())) {
            Ok(()) => break,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "registered tree cleanup remains incomplete: {error}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    assert_eq!(
        std::fs::symlink_metadata(&root).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    directory.complete::<()>(Ok(())).unwrap();
}

#[test]
fn product_cleanup_removes_moved_original_and_keeps_foreign_replacement() {
    let mut probe = NativeProbeTestBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut probe).unwrap();
    let original = directory.path().to_owned();
    let nested = original.join("nested");
    directory.create_dir_all(&nested, &mut probe).unwrap();
    directory
        .write(&nested.join("owned"), b"original", &mut probe)
        .unwrap();
    let moved = original.with_extension("moved");
    rename_after_concurrent_creation(&original, &moved);
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("foreign"), b"foreign payload").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match directory.complete::<()>(Ok(())) {
            Ok(()) => break,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "original cleanup remains incomplete: {error}"
                );
                assert_eq!(
                    std::fs::read(original.join("foreign")).unwrap(),
                    b"foreign payload"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    assert_eq!(
        std::fs::symlink_metadata(&moved).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(
        std::fs::read(original.join("foreign")).unwrap(),
        b"foreign payload"
    );
    directory.complete::<()>(Ok(())).unwrap();
    std::fs::remove_file(original.join("foreign")).unwrap();
    std::fs::remove_dir(original).unwrap();
}

#[test]
fn product_cleanup_keeps_unregistered_foreign_children_and_original_owner() {
    let mut probe = NativeProbeTestBudget::new(&ProbeLimits::default()).unwrap();
    let mut directory = GitPrivateDirectory::with_limits(128 << 20, 0, &mut probe).unwrap();
    let root = directory.path().to_owned();
    let foreign = root.join("foreign");
    let moved = root.join("moved-foreign");
    std::fs::write(&foreign, b"not registered").unwrap();
    assert!(directory.complete::<()>(Ok(())).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"not registered");
    std::fs::rename(&foreign, &moved).unwrap();
    std::fs::write(&foreign, b"foreign replacement").unwrap();
    assert!(directory.complete::<()>(Ok(())).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign replacement");
    assert_eq!(std::fs::read(&moved).unwrap(), b"not registered");
    // 测试 actor 收回自己的两份外来数据；产品必须证明原完整 ID 消失后清理原登记根。
    std::fs::remove_file(foreign).unwrap();
    assert!(directory.complete::<()>(Ok(())).is_err());
    assert_eq!(std::fs::read(&moved).unwrap(), b"not registered");
    std::fs::remove_file(moved).unwrap();
    while let Err(error) = directory.complete::<()>(Ok(())) {
        probe.check().unwrap_or_else(|deadline| {
            panic!("original foreign recovery budget exhausted: {deadline}; last error: {error}")
        });
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !root.exists(),
        "actual original owner must delete its original root"
    );
    directory.complete::<()>(Ok(())).unwrap();
    println!("DG_WINDOWS_FOREIGN_ENTRY_PRODUCT_RECOVERY=1");
}

/// 并行夹具共享系统临时父目录；只等待短期创建租约，持续冲突仍失败。
fn rename_after_concurrent_creation(from: &std::path::Path, to: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return,
            Err(error) => {
                assert_eq!(
                    error.raw_os_error(),
                    Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32),
                    "rename failed for a reason other than a concurrent creation lease: {error}"
                );
                assert!(
                    Instant::now() < deadline,
                    "creation lease did not release before actual rename: {error}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn original_root_cross_parent_cleanup_preserves_foreign_replacement() {
    use super::git_directory_lease::GitDirectoryLease;
    use super::git_directory_security::GitDirectorySecurity;
    use super::probe_budget::ProbeBudget;
    use super::windows_git_cleanup::WindowsGitCleanup;
    use super::windows_git_private_root::WindowsGitPrivateRoot;
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let original = parent.join("original");
    let destination = parent.join("destination");
    std::fs::create_dir(&destination).unwrap();
    let moved = destination.join("moved");
    let mut probe = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let lease = GitDirectoryLease::open(&parent, &mut probe).unwrap();
    let mut owner =
        WindowsGitCleanup::new(lease.leaf_file(), original.clone(), &mut probe).unwrap();
    WindowsGitPrivateRoot::create_into(
        lease.leaf_file(),
        std::ffi::OsStr::new("original"),
        &GitDirectorySecurity::new().unwrap(),
        &mut owner.root,
    )
    .unwrap();
    drop(lease);
    std::fs::rename(&original, &moved).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join("foreign"), b"foreign sentinel").unwrap();
    println!("DG_CROSS_PARENT_ORIGINAL_ROOT_RED_READY=1");
    let result = loop {
        let result = owner.cleanup(None);
        if result.is_ok() || probe.check().is_err() {
            break result;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        result.is_ok(),
        "actual cross-parent original root removal must complete: {result:?}"
    );
    assert!(!moved.exists());
    assert_eq!(
        std::fs::read(original.join("foreign")).unwrap(),
        b"foreign sentinel"
    );
    println!("DG_CROSS_PARENT_ORIGINAL_ROOT_GREEN=1");
}
