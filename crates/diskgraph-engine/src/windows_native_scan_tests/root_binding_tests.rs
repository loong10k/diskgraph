//! D31 当前 namespace 绑定与保留句柄身份必须同时成立。
use diskgraph_core::{BusinessError, NodeKind};

use crate::EngineError;
use crate::windows_native_scan_root::WindowsNativeScanRoot;

use super::{node, root, settings};

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
