//! D31 原生 Windows 夹具；本模块必须由 Windows 宿主实际执行。
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use diskgraph_core::{
    BusinessError, DiskNode, NodeKind, ResourceLocator, ScanSettings, WindowsObservationGap,
    WindowsTreeAlignment,
};

use crate::EngineError;
use crate::windows_native_scan_root::WindowsNativeScanRoot;

mod root_binding_tests;

thread_local! {
    static BETWEEN_CAPTURES: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

pub(crate) fn between_captures() {
    let hook = BETWEEN_CAPTURES.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

struct CaptureHook;

impl CaptureHook {
    fn install(hook: impl FnOnce() + 'static) -> Self {
        BETWEEN_CAPTURES.with(|slot| {
            assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
        });
        Self
    }
}

impl Drop for CaptureHook {
    fn drop(&mut self) {
        BETWEEN_CAPTURES.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

fn settings() -> ScanSettings {
    ScanSettings {
        apparent_size: true,
        follow_links: false,
        include_hidden: true,
        one_filesystem: true,
        max_depth: None,
        dedup_hardlinks: false,
    }
}

fn node(path: &Path, kind: NodeKind, len: u64) -> DiskNode {
    DiskNode {
        id: 1,
        parent_id: None,
        locator: ResourceLocator::NativePath(path.to_string_lossy().into_owned()),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind,
        subtree_bytes: len,
        direct_bytes: len,
        size_known: true,
        files: u64::from(kind == NodeKind::File),
        directories: u64::from(kind == NodeKind::Directory),
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    }
}

fn root(workspace: &tempfile::TempDir) -> PathBuf {
    workspace.path().canonicalize().unwrap()
}

fn junction(link: &Path, target: &Path) {
    let result = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(command_path(link))
        .arg(command_path(target))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "junction fixture failed: {result:?}"
    );
}

// 仅夹具外部工具参数：受控 TempDir 的普通名称保留同一 drive 路径，原生能力仍用原路径。
fn command_path(path: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let units = if units.starts_with(&[92, 92, 63, 92]) {
        assert!(units.len() >= 7 && units[5] == 58 && units[6] == 92);
        &units[4..]
    } else {
        &units
    };
    PathBuf::from(OsString::from_wide(units))
}

#[test]
fn native_scan_observes_root_directory_file_and_drive_root() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let path = root.join("plain.bin");
    std::fs::write(&path, b"plain").unwrap();
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let (directory, gap) = lease
        .observe(
            &root,
            &node(&root, NodeKind::Directory, 5),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap();
    assert!(gap.is_none());
    let directory = directory.unwrap();
    assert!(directory.directory);
    assert_eq!(directory.tree_alignment, WindowsTreeAlignment::Unverified);
    let (file, gap) = lease
        .observe(
            &path,
            &node(&path, NodeKind::File, 5),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap();
    assert!(gap.is_none());
    let file = file.unwrap();
    assert_eq!(file.length, 5);
    assert_eq!(file.volume, directory.volume);
    assert!(file.capture_started_unix_ms <= file.capture_finished_unix_ms);
    assert_eq!(file.encode().unwrap().len(), 80);
    lease.validate_root(&|| Ok(())).unwrap();
    let drive = crate::windows_path_plan::WindowsPathPlan::for_root(&root)
        .unwrap()
        .drive_root;
    let drive_lease = WindowsNativeScanRoot::open(&drive, &|| Ok(())).unwrap();
    let (observed, gap) = drive_lease
        .observe(
            &drive,
            &node(&drive, NodeKind::Directory, 0),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap();
    assert!(observed.unwrap().directory);
    assert!(gap.is_none());
    drive_lease.validate_root(&|| Ok(())).unwrap();
}

#[test]
fn native_hardlinks_keep_full_identity_and_replacement_is_tree_mismatch() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let path = root.join("source");
    let link = root.join("hardlink");
    std::fs::write(&path, b"old").unwrap();
    std::fs::hard_link(&path, &link).unwrap();
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let first = lease
        .observe(
            &path,
            &node(&path, NodeKind::File, 3),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap()
        .0
        .unwrap();
    let second = lease
        .observe(
            &link,
            &node(&link, NodeKind::File, 3),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap()
        .0
        .unwrap();
    assert_eq!(
        (first.volume, first.file_id),
        (second.volume, second.file_id)
    );
    let mut old = node(&path, NodeKind::File, 3);
    old.file_identity = Some(
        first
            .legacy_identity()
            .expect("NTFS fixture needs lossless legacy ID"),
    );
    let matched = lease
        .observe(&path, &old, None, &settings(), &|| Ok(()))
        .unwrap()
        .0
        .unwrap();
    assert_eq!(matched.tree_alignment, WindowsTreeAlignment::Matched);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"new").unwrap();
    let (observation, gap) = lease
        .observe(&path, &old, None, &settings(), &|| Ok(()))
        .unwrap();
    assert!(observation.is_none());
    assert_eq!(gap, Some(WindowsObservationGap::TreeMismatch));
    assert_eq!(std::fs::read(&link).unwrap(), b"old");
}

#[test]
fn native_alignment_refuses_type_size_and_unknown_size_assumptions() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let path = root.join("plain");
    std::fs::write(&path, b"plain").unwrap();
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let first = lease
        .observe(
            &path,
            &node(&path, NodeKind::File, 5),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap()
        .0
        .unwrap();
    let mut expected = node(&path, NodeKind::File, 5);
    expected.file_identity = first.legacy_identity();
    for (apparent, dedup, known) in [
        (false, false, true),
        (true, true, true),
        (true, false, false),
    ] {
        let mut policy = settings();
        policy.apparent_size = apparent;
        policy.dedup_hardlinks = dedup;
        expected.size_known = known;
        expected.direct_bytes = 0;
        let actual = lease
            .observe(&path, &expected, None, &policy, &|| Ok(()))
            .unwrap()
            .0
            .unwrap();
        assert_eq!(actual.tree_alignment, WindowsTreeAlignment::Unverified);
    }
    expected.size_known = true;
    let (_, gap) = lease
        .observe(&path, &expected, None, &settings(), &|| Ok(()))
        .unwrap();
    assert_eq!(gap, Some(WindowsObservationGap::TreeMismatch));
    expected.kind = NodeKind::Directory;
    let (_, gap) = lease
        .observe(&path, &expected, None, &settings(), &|| Ok(()))
        .unwrap();
    assert_eq!(gap, Some(WindowsObservationGap::TreeMismatch));
}

#[test]
fn junction_leaf_is_observed_without_following_and_parent_junction_is_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let outside = tempfile::tempdir().unwrap();
    let outside_root = outside.path().canonicalize().unwrap();
    std::fs::write(outside_root.join("secret"), b"outside").unwrap();
    let link = root.join("junction");
    junction(&link, &outside_root);
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let (observed, gap) = lease
        .observe(
            &link,
            &node(&link, NodeKind::Symlink, 0),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap();
    assert!(gap.is_none());
    assert!(observed.unwrap().attributes & 0x400 != 0);
    let escaped = link.join("secret");
    let (observed, gap) = lease
        .observe(
            &escaped,
            &node(&escaped, NodeKind::File, 7),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap();
    assert!(observed.is_none());
    assert_eq!(gap, Some(WindowsObservationGap::Unsupported));
    assert!(WindowsNativeScanRoot::open(&link, &|| Ok(())).is_err());
    drop(lease);
    std::fs::remove_dir(&link).unwrap();
}

#[test]
fn relative_planning_rejects_outside_root_dot_ads_namespace_and_case_guessing() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    for path in [
        PathBuf::from(r"\\server\share\file"),
        root.join("x:stream"),
        root.with_file_name("outside"),
    ] {
        let (observed, gap) = lease
            .observe(
                &path,
                &node(&path, NodeKind::File, 0),
                None,
                &settings(),
                &|| Ok(()),
            )
            .unwrap();
        assert!(observed.is_none());
        assert_eq!(gap, Some(WindowsObservationGap::Unsupported));
    }
    let mut dot = root.as_os_str().to_os_string();
    dot.push(r"\..\outside");
    let dot = PathBuf::from(dot);
    assert!(crate::windows_path_plan::WindowsPathPlan::relative_to_root(&root, &dot).is_err());
    let changed_case = PathBuf::from(root.to_string_lossy().to_uppercase());
    if changed_case != root {
        assert!(
            crate::windows_path_plan::WindowsPathPlan::relative_to_root(&root, &changed_case)
                .is_err()
        );
    }
}

#[test]
fn same_handle_change_between_captures_is_not_confirmed() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let path = root.join("changing");
    std::fs::write(&path, b"old").unwrap();
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let write_path = path.clone();
    let _hook =
        CaptureHook::install(move || std::fs::write(write_path, b"changed length").unwrap());
    let (observed, gap) = lease
        .observe(
            &path,
            &node(&path, NodeKind::File, 3),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap();
    assert!(observed.is_none());
    assert_eq!(gap, Some(WindowsObservationGap::Changed));
}

#[test]
fn cancellation_deadline_authorization_and_fence_errors_are_not_gap_labels() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let path = root.join("plain");
    std::fs::write(&path, b"plain").unwrap();
    for error in [
        BusinessError::Partial,
        BusinessError::Timeout,
        BusinessError::PermissionDenied,
        BusinessError::Conflict,
    ] {
        assert!(
            matches!(WindowsNativeScanRoot::open(&root, &|| Err(error.into())), Err(EngineError::Business(actual)) if actual == error)
        );
        let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let changed = stopped.clone();
        let _hook = CaptureHook::install(move || changed.store(true, Ordering::Release));
        let check = || {
            if stopped.load(Ordering::Acquire) {
                Err(EngineError::Business(error))
            } else {
                Ok(())
            }
        };
        assert!(
            matches!(lease.observe(&path, &node(&path, NodeKind::File, 5), None, &settings(), &check), Err(EngineError::Business(actual)) if actual == error)
        );
        assert!(
            matches!(lease.validate_root(&check), Err(EngineError::Business(actual)) if actual == error)
        );
    }
}

#[test]
fn attribute_observation_does_not_require_read_data_permission() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let path = root.join("attributes-only");
    std::fs::write(&path, b"secret").unwrap();
    let denied = std::process::Command::new("icacls.exe")
        .arg(command_path(&path))
        .args(["/deny", "*S-1-1-0:(RD)"])
        .output()
        .unwrap();
    assert!(
        denied.status.success(),
        "data ACL fixture failed: {denied:?}"
    );
    let data_denied = std::fs::File::open(&path).is_err();
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let result = lease.observe(
        &path,
        &node(&path, NodeKind::File, 6),
        None,
        &settings(),
        &|| Ok(()),
    );
    let restored = std::process::Command::new("icacls.exe")
        .arg(command_path(&path))
        .args(["/remove:d", "*S-1-1-0"])
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "restore ACL failed: {restored:?}"
    );
    assert!(data_denied, "positive control must forbid data reads");
    let (observed, gap) = result.unwrap();
    assert!(gap.is_none());
    assert_eq!(observed.unwrap().length, 6);
}

#[test]
fn native_state_checks_between_each_query_and_preserves_caller_error() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("native-query");
    std::fs::write(&path, b"native").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let calls = std::cell::Cell::new(0);
    let check = || {
        calls.set(calls.get() + 1);
        if calls.get() == 4 {
            Err(EngineError::Business(BusinessError::PermissionDenied))
        } else {
            Ok(())
        }
    };
    assert!(matches!(
        crate::windows_file_state::WindowsFileState::capture_checked(&file, &check),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert_eq!(calls.get(), 4);
    calls.set(0);
    let check = || {
        calls.set(calls.get() + 1);
        Ok(())
    };
    let state = crate::windows_file_state::WindowsFileState::capture_checked(&file, &check)
        .unwrap()
        .unwrap();
    assert_eq!(state.len, 6);
    assert_eq!(calls.get(), 8, "each of four native queries has two checks");
}
