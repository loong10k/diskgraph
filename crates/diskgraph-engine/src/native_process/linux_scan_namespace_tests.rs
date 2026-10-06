//! 原目录身份缓存的真实 Linux 名称绑定回归；来源：Rust FS-02 / 临时 tmpfs 元数据。
use super::linux_scan_namespace::LinuxScanNamespace;
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::cell::Cell;
use std::os::unix::fs::{MetadataExt, symlink};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir_in("/dev/shm").expect("native Linux tmpfs fixture required");
    let root = directory.path().join("container/scope");
    std::fs::create_dir_all(&root).unwrap();
    (directory, root)
}

#[test]
fn original_directory_fd_survives_replacement_but_current_binding_is_rejected() {
    let (directory, root) = fixture();
    let held = LinuxScanNamespace::open(&root, &|| Ok(()))
        .expect("actual openat2/statx unique-mount capability is required, not skipped");
    held.verify(&|| Ok(())).unwrap();
    let original = held.root().metadata().unwrap();
    std::fs::rename(&root, directory.path().join("retained-original")).unwrap();
    std::fs::create_dir(&root).unwrap();
    let retained = held.root().metadata().unwrap();
    assert_eq!(
        (retained.dev(), retained.ino()),
        (original.dev(), original.ino())
    );
    assert_ne!(std::fs::metadata(&root).unwrap().ino(), original.ino());
    assert_eq!(held.verify(&|| Ok(())), Err(Failure::Conflict));
}

#[test]
fn cached_original_identity_does_not_cache_the_current_ancestor_route() {
    let (directory, root) = fixture();
    let held = LinuxScanNamespace::open(&root, &|| Ok(())).unwrap();
    held.verify(&|| Ok(())).unwrap();
    let original = held.root().metadata().unwrap();
    let container = directory.path().join("container");
    let old_container = directory.path().join("old-container");
    let ancestor_inode = std::fs::metadata(&container).unwrap().ino();
    std::fs::rename(&container, &old_container).unwrap();
    std::fs::create_dir(&container).unwrap();
    std::fs::rename(old_container.join("scope"), &root).unwrap();
    let current = std::fs::metadata(&root).unwrap();
    assert_eq!(
        (current.dev(), current.ino()),
        (original.dev(), original.ino())
    );
    assert_ne!(std::fs::metadata(container).unwrap().ino(), ancestor_inode);
    assert_eq!(held.verify(&|| Ok(())), Err(Failure::Conflict));
}

#[test]
fn repeated_current_verification_allows_sibling_metadata_changes() {
    let (directory, root) = fixture();
    let held = LinuxScanNamespace::open(&root, &|| Ok(())).unwrap();
    let original = held.root().metadata().unwrap();
    for index in 0..3 {
        let sibling = directory.path().join(format!("sibling-{index}"));
        std::fs::create_dir(&sibling).unwrap();
        held.verify(&|| Ok(())).unwrap();
        std::fs::remove_dir(sibling).unwrap();
        held.verify(&|| Ok(())).unwrap();
    }
    let after = held.root().metadata().unwrap();
    assert_eq!((after.dev(), after.ino()), (original.dev(), original.ino()));
}

#[test]
fn current_route_symlink_is_not_accepted_even_when_it_resolves_to_original_inode() {
    let (directory, root) = fixture();
    let held = LinuxScanNamespace::open(&root, &|| Ok(())).unwrap();
    let retained = directory.path().join("retained-original");
    std::fs::rename(&root, &retained).unwrap();
    symlink(&retained, &root).unwrap();
    assert_eq!(
        std::fs::metadata(&root).unwrap().ino(),
        held.root().metadata().unwrap().ino()
    );
    assert_eq!(held.verify(&|| Ok(())), Err(Failure::Unsupported));
}

#[test]
fn current_verification_preserves_a_denial_after_the_initial_check() {
    let (_directory, root) = fixture();
    let held = LinuxScanNamespace::open(&root, &|| Ok(())).unwrap();
    let calls = Cell::new(0);
    let result = held.verify(&|| {
        calls.set(calls.get() + 1);
        if calls.get() > 1 {
            Err(Failure::PermissionDenied)
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err(Failure::PermissionDenied));
    assert_eq!(calls.get(), 2);
    held.verify(&|| Ok(())).unwrap();
}

#[test]
fn injected_transient_race_then_actual_constrained_open_retains_original_identity() {
    use super::bounded_open_retry::bounded_open_retry;
    use super::linux_open::open_at_io;
    use std::os::fd::AsRawFd;
    let (_directory, root) = fixture();
    let held = LinuxScanNamespace::open(&root, &|| Ok(())).unwrap();
    let calls = Cell::new(0);
    let file = bounded_open_retry(
        &|| Ok(()),
        &mut || {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                // 首次竞态为注入控制；第二次必须真的调用带原解析约束的 openat2。
                Err(std::io::Error::from_raw_os_error(libc::EAGAIN))
            } else {
                open_at_io(
                    held.root().as_raw_fd(),
                    c".",
                    libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                    0x08 | 0x04 | 0x02 | 0x20,
                )
            }
        },
        libc::EAGAIN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(calls.get(), 2);
    let original = held.root().metadata().unwrap();
    let current = file.metadata().unwrap();
    assert_eq!(
        (original.dev(), original.ino()),
        (current.dev(), current.ino())
    );
    held.verify(&|| Ok(())).unwrap();
}
