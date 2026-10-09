//! D31 当前 namespace 绑定与保留句柄身份必须同时成立。
use diskgraph_core::{BusinessError, NodeKind};

use crate::EngineError;
use crate::windows_native_scan_root::WindowsNativeScanRoot;

use super::{node, root, settings};

#[test]
fn repeated_observations_parse_only_the_requested_path() {
    use crate::windows_path_plan::WindowsPathPlan;

    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let paths: Vec<_> = (0..20).map(|i| root.join(format!("node-{i}"))).collect();
    for path in &paths {
        std::fs::write(path, b"owned").unwrap();
    }
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let before = WindowsPathPlan::parse_work_for_tests();
    for path in &paths {
        let (observed, gap) = lease
            .observe(
                path,
                &node(path, NodeKind::File, 5),
                None,
                &settings(),
                &|| Ok(()),
            )
            .unwrap();
        assert!(observed.is_some());
        assert!(gap.is_none());
    }
    assert_eq!(
        WindowsPathPlan::parse_work_for_tests() - before,
        paths.len(),
        "fixed registered root must not be re-parsed for every node"
    );
}

#[test]
fn preencoded_component_keeps_identity_and_rejects_invalid_names() {
    use crate::windows_file_state::WindowsFileState;
    use crate::windows_native_open::{open_child, open_child_utf16, open_drive};
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace);
    let plan = crate::windows_path_plan::WindowsPathPlan::for_root(&root).unwrap();
    let mut parent = open_drive(&plan.drive_root).unwrap();
    for name in &plan.components {
        let encoded: Vec<u16> = name.encode_wide().collect();
        let ordinary = open_child(&parent, name, true).unwrap();
        let prepared = open_child_utf16(&parent, &encoded, true).unwrap();
        assert_eq!(
            WindowsFileState::capture(&ordinary).unwrap(),
            WindowsFileState::capture(&prepared).unwrap()
        );
        parent = prepared;
    }
    for invalid in [
        vec![],
        vec![46],
        vec![46, 46],
        vec![47],
        vec![58],
        vec![92],
        vec![0],
        vec![b'x' as u16; 32768],
    ] {
        assert!(matches!(
            open_child_utf16(&parent, &invalid, true),
            Err(EngineError::Business(BusinessError::InvalidArgument))
        ));
    }
    // 原生名称不经过UTF-8替换；包括未配对surrogate的实际文件。
    for encoded in [vec![0x4e2d, 0x6587], vec![0x78, 0xd800]] {
        let name = OsString::from_wide(&encoded);
        std::fs::write(root.join(&name), b"native").unwrap();
        let ordinary = open_child(&parent, &name, false).unwrap();
        let prepared = open_child_utf16(&parent, &encoded, false).unwrap();
        assert_eq!(
            WindowsFileState::capture(&ordinary).unwrap(),
            WindowsFileState::capture(&prepared).unwrap()
        );
    }
}

#[test]
fn held_root_allows_directory_time_change_without_replacement() {
    let workspace = tempfile::tempdir().unwrap();
    let parent = root(&workspace);
    let root = parent.join("scope");
    std::fs::create_dir(&root).unwrap();
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    std::fs::write(root.join("later"), b"later").unwrap();
    lease.validate_root(&|| Ok(())).unwrap();
    let (observed, gap) = lease
        .observe(
            &root,
            &node(&root, NodeKind::Directory, 5),
            None,
            &settings(),
            &|| Ok(()),
        )
        .unwrap();
    assert!(gap.is_none());
    assert!(observed.unwrap().directory);
}

#[test]
fn renamed_and_rebound_registered_root_is_conflict() {
    assert_directory_rebinding_is_conflict(false);
}

#[test]
fn renamed_and_rebound_ancestor_is_conflict() {
    assert_directory_rebinding_is_conflict(true);
}

fn assert_directory_rebinding_is_conflict(replace_ancestor: bool) {
    let workspace = tempfile::tempdir().unwrap();
    let parent = root(&workspace);
    let ancestor = parent.join("ancestor");
    let root = ancestor.join("scope");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("observed");
    std::fs::write(&path, b"owned").unwrap();
    let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
    let original = lease
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
    let replaced = if replace_ancestor { &ancestor } else { &root };
    let moved = parent.join("moved");
    match std::fs::rename(replaced, &moved) {
        Ok(()) => {
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(&path, b"foreign").unwrap();
            // 在两个断言前实际执行两条路径，避免首个失败遮蔽另一个旧行为。
            let validation = lease.validate_root(&|| Ok(()));
            let observation = lease.observe(
                &path,
                &node(&path, NodeKind::File, 5),
                None,
                &settings(),
                &|| Ok(()),
            );
            assert!(
                matches!(
                    validation,
                    Err(EngineError::Business(BusinessError::Conflict))
                ),
                "renamed binding must invalidate publication: {validation:?}"
            );
            assert!(
                matches!(
                    observation,
                    Err(EngineError::Business(BusinessError::Conflict))
                ),
                "renamed binding must invalidate sampling: {observation:?}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"foreign");
        }
        Err(error) => {
            eprintln!("filesystem enforced stronger rename exclusion while lease is held: {error}");
            lease.validate_root(&|| Ok(())).unwrap();
            let current = lease
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
            assert_eq!(
                (original.volume, original.file_id),
                (current.volume, current.file_id)
            );
            drop(lease);
            // 夹具本身须有替换权限；不能把任意环境失败当有效防护或跳过。
            std::fs::rename(replaced, &moved).unwrap();
        }
    }
}
