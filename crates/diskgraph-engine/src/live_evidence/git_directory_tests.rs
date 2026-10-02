use super::ProbeLimits;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_directory::GitMetadataDirectory;
use super::git_private_directory::GitPrivateDirectory;
use super::probe_budget::ProbeBudget;
use std::ffi::OsString;
use std::sync::atomic::Ordering;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    (temp, root)
}

fn probe() -> ProbeBudget {
    ProbeBudget::new(&ProbeLimits::default()).unwrap()
}

#[test]
fn private_directory_is_unique_absolute_and_removed_on_drop() {
    let mut probe = probe();
    let first = GitPrivateDirectory::new(&mut probe).unwrap();
    let second = GitPrivateDirectory::new(&mut probe).unwrap();
    assert!(first.path().is_absolute());
    assert_ne!(first.path(), second.path());
    std::fs::write(first.path().join("owned"), b"private").unwrap();
    let path = first.path().to_owned();
    drop(first);
    assert!(!path.exists());
    assert!(second.path().is_dir());
}

#[cfg(unix)]
#[test]
fn private_directory_permissions_do_not_admit_other_users() {
    use std::os::unix::fs::PermissionsExt;
    let directory = GitPrivateDirectory::new(&mut probe()).unwrap();
    assert_eq!(
        std::fs::metadata(directory.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn directory_names_are_sorted_leaf_components_and_charge_each_entry() {
    let (_temp, root) = fixture();
    std::fs::write(root.as_path().join("z"), b"z").unwrap();
    std::fs::write(root.as_path().join("a"), b"a").unwrap();
    let mut probe = probe();
    let mut budget = GitMetadataBudget::new(64, 3).unwrap();
    let directory = GitMetadataDirectory::capture(root.as_path(), &mut budget, &mut probe).unwrap();
    assert_eq!(directory.path(), root.as_path());
    assert_eq!(
        directory.names(),
        [OsString::from("a"), OsString::from("z")]
    );
    assert!(
        budget.charge_entry(&mut probe).is_err(),
        "directory and both leaf entries must consume quota"
    );
}

#[test]
fn directory_capture_does_not_exceed_name_bytes_or_entry_quota() {
    let (_temp, root) = fixture();
    std::fs::write(root.as_path().join("long-name"), b"").unwrap();
    assert!(
        GitMetadataDirectory::capture(
            root.as_path(),
            &mut GitMetadataBudget::new(0, 10).unwrap(),
            &mut probe()
        )
        .is_err()
    );
    assert!(
        GitMetadataDirectory::capture(
            root.as_path(),
            &mut GitMetadataBudget::new(64, 1).unwrap(),
            &mut probe()
        )
        .is_err()
    );
}

#[test]
fn directory_identity_replacement_cannot_pass_same_name_verification() {
    let (_temp, root) = fixture();
    let path = root.as_path().join("metadata");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("HEAD"), b"one").unwrap();
    let mut probe = probe();
    let mut budget = GitMetadataBudget::default();
    let directory = GitMetadataDirectory::capture(&path, &mut budget, &mut probe).unwrap();
    let replaced = std::fs::rename(&path, root.as_path().join("old"));
    #[cfg(windows)]
    if let Err(error) = replaced {
        // shareREAD 目录租约可由系统直接拒绝替换；未发生替换时原记录应仍可复核。
        assert_eq!(error.raw_os_error(), Some(32));
        directory.verify(&mut budget, &mut probe).unwrap();
        return;
    }
    #[cfg(not(windows))]
    replaced.unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("HEAD"), b"one").unwrap();
    assert!(directory.verify(&mut budget, &mut probe).is_err());
}

#[cfg(unix)]
#[test]
fn directory_capture_rejects_leaf_and_parent_links_without_canonicalizing() {
    use std::os::unix::fs::symlink;
    let (_temp, root) = fixture();
    let real = root.as_path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::create_dir(real.join("child")).unwrap();
    let link = root.as_path().join("link");
    symlink(&real, &link).unwrap();
    for path in [link.clone(), link.join("child")] {
        assert!(
            GitMetadataDirectory::capture(&path, &mut GitMetadataBudget::default(), &mut probe())
                .is_err(),
            "followed {}",
            path.display()
        );
    }
}

#[test]
fn directory_capture_rejects_relative_parent_and_regular_file_paths() {
    let (_temp, root) = fixture();
    let file = root.as_path().join("file");
    std::fs::write(&file, b"").unwrap();
    for path in [
        std::path::PathBuf::from("."),
        root.as_path().join("../"),
        file,
    ] {
        assert!(
            GitMetadataDirectory::capture(&path, &mut GitMetadataBudget::default(), &mut probe())
                .is_err(),
            "accepted {}",
            path.display()
        );
    }
}

#[test]
fn directory_operations_honor_preexisting_cancellation() {
    let limits = ProbeLimits::default();
    limits.cancel.store(true, Ordering::Release);
    let mut cancelled = ProbeBudget::new(&limits).unwrap();
    let (_temp, root) = fixture();
    assert!(GitPrivateDirectory::new(&mut cancelled).is_err());
    assert!(
        GitMetadataDirectory::capture(
            root.as_path(),
            &mut GitMetadataBudget::default(),
            &mut cancelled
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn directory_names_preserve_non_utf8_native_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let (_temp, root) = fixture();
    let name = OsString::from_vec(vec![b'n', 0xff]);
    // Darwin 部分卷不支持非法 UTF-8 名称；Linux 原生门禁必须执行本夹具。
    if let Err(error) = std::fs::write(root.as_path().join(&name), b"") {
        if cfg!(target_os = "macos") && error.raw_os_error() == Some(libc::EILSEQ) {
            return;
        }
        panic!("native name fixture: {error}");
    }
    let directory = GitMetadataDirectory::capture(
        root.as_path(),
        &mut GitMetadataBudget::default(),
        &mut probe(),
    )
    .unwrap();
    assert_eq!(directory.names(), [name]);
}

#[test]
fn directory_unchanged_verification_and_missing_name_proof() {
    let (_temp, root) = fixture();
    let path = root.join("metadata");
    std::fs::create_dir(&path).unwrap();
    let mut budget = GitMetadataBudget::default();
    let mut probe = probe();
    let directory = GitMetadataDirectory::capture(&path, &mut budget, &mut probe).unwrap();
    assert!(!directory.names().contains(&OsString::from("refs")));
    directory.verify(&mut budget, &mut probe).unwrap();
    std::fs::create_dir(path.join("refs")).unwrap();
    assert!(directory.verify(&mut budget, &mut probe).is_err());
}

#[cfg(unix)]
#[test]
fn directory_parent_replacement_is_detected_with_original_leaf_inode() {
    let (_temp, root) = fixture();
    let parent = root.join("parent");
    let path = parent.join("metadata");
    std::fs::create_dir_all(&path).unwrap();
    let mut budget = GitMetadataBudget::default();
    let mut probe = probe();
    let directory = GitMetadataDirectory::capture(&path, &mut budget, &mut probe).unwrap();
    std::fs::rename(&parent, root.join("old")).unwrap();
    std::fs::create_dir(&parent).unwrap();
    std::fs::rename(root.join("old/metadata"), &path).unwrap();
    assert!(directory.verify(&mut budget, &mut probe).is_err());
}

#[test]
fn directory_verification_honors_later_cancellation() {
    let (_temp, root) = fixture();
    let limits = ProbeLimits::default();
    let mut probe = ProbeBudget::new(&limits).unwrap();
    let mut budget = GitMetadataBudget::default();
    let directory = GitMetadataDirectory::capture(&root, &mut budget, &mut probe).unwrap();
    limits.cancel.store(true, Ordering::Release);
    assert!(directory.verify(&mut budget, &mut probe).is_err());
}

#[cfg(any(unix, windows))]
#[test]
fn directory_records_do_not_accumulate_open_descriptors() {
    const CHILD: &str = "DISKGRAPH_DIRECTORY_FD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "live_evidence::git_directory_tests::directory_records_do_not_accumulate_open_descriptors", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "isolated descriptor budget: {output:?}"
        );
        return;
    }
    let (_temp, root) = fixture();
    let paths: Vec<_> = (0..64)
        .map(|number| root.join(format!("directory-{number}")))
        .collect();
    for path in &paths {
        std::fs::create_dir(path).unwrap();
        std::fs::write(path.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
    }
    #[cfg(unix)]
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    #[cfg(unix)]
    {
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) },
            0
        );
        limits.rlim_cur = 64.min(limits.rlim_max);
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) }, 0);
    }
    #[cfg(unix)]
    let live_count = || {
        (0..limits.rlim_cur as i32)
            .filter(|fd| unsafe { libc::fcntl(*fd, libc::F_GETFD) } >= 0)
            .count()
    };
    #[cfg(windows)]
    let live_count = || {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
        let mut count = 0;
        assert_ne!(
            unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) },
            0
        );
        count as usize
    };
    let before = live_count();
    let mut budget = GitMetadataBudget::default();
    let mut probe = probe();
    let records: Vec<_> = paths
        .iter()
        .map(|path| {
            GitMetadataDirectory::capture(path, &mut budget, &mut probe).unwrap_or_else(|error| {
                panic!(
                    "capture {} without accumulating handles: {error}",
                    path.display()
                )
            })
        })
        .collect();
    assert_eq!(live_count(), before, "captured records retain descriptors");
    for record in &records {
        record.verify(&mut budget, &mut probe).unwrap();
    }
    assert_eq!(live_count(), before, "verification leaks descriptors");
    assert_eq!(records.len(), 64);
}

#[cfg(windows)]
#[test]
fn private_directory_dacl_is_protected_and_inherited_by_files_and_directories() {
    let directory = GitPrivateDirectory::new(&mut probe()).unwrap();
    let file = directory.path().join("private-file");
    let child = directory.path().join("private-child");
    std::fs::write(&file, b"private").unwrap();
    std::fs::create_dir(&child).unwrap();
    let (root, protected) = directory_dacl(directory.path());
    assert!(
        protected,
        "creation must protect the directory DACL: {root}"
    );
    assert_eq!(root.matches("(A;").count(), 2, "{root}");
    assert!(root.contains("OICI"), "{root}");
    assert!(root.contains(";;;SY)"), "{root}");
    for path in [file, child] {
        let (inherited, _) = directory_dacl(&path);
        assert_eq!(inherited.matches("(A;").count(), 2, "{inherited}");
        assert!(inherited.contains("ID"), "{inherited}");
        assert!(inherited.contains(";;;SY)"), "{inherited}");
        for broad in [";;;WD)", ";;;BU)", ";;;AU)"] {
            assert!(!inherited.contains(broad), "{inherited}");
        }
    }
}

#[cfg(windows)]
fn directory_dacl(path: &std::path::Path) -> (String, bool) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, SE_DACL_PROTECTED,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        },
        0
    );
    let mut control = 0;
    let mut revision = 0;
    let queried = unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) };
    let mut text = std::ptr::null_mut();
    let mut length = 0;
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            &mut length,
        )
    };
    unsafe { LocalFree(descriptor.cast()) };
    assert_ne!(queried, 0);
    assert_ne!(converted, 0);
    let units = unsafe { std::slice::from_raw_parts(text, length as usize) };
    let value = String::from_utf16_lossy(units)
        .trim_end_matches('\0')
        .to_owned();
    unsafe { LocalFree(text.cast()) };
    (value, control & SE_DACL_PROTECTED != 0)
}
